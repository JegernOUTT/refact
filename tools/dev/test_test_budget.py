"""Focused stdlib tests, isolated repositories; never build or mutate Rust workspace."""
import contextlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

import test_budget as gate


EMPTY = {'version': 1, 'tests': [], 'impacts': [], 'classifications': []}


class ScannerTests(unittest.TestCase):
    def scan(self, source):
        return gate.scan('orphan.rs', source.encode())

    def test_comments_strings_nested_comments_not_tests(self):
        src = '''/* outer /* #[test] fn fake() {} */ end */
        const X: &str = r###"#[test] fn fake() {}"###;
        fn helper<'a>(s: &'a str) { let x = '\\''; }
        #[ignore] #[cfg(any())] #[tokio::test] async fn real() { assert_eq!("x", "x"); }
        '''
        tests, _, errors, _ = self.scan(src)
        self.assertEqual([x['name'] for x in tests], ['real'])
        self.assertEqual(errors, [])

    def test_nested_cfg_attr_namespaces(self):
        tests, _, _, _ = self.scan('#[cfg_attr(test, cfg_attr(feature="x", foo::test))] fn t() {}')
        self.assertEqual(len(tests), 1)
        self.assertEqual(self.scan('#[cfg_attr(test, allow(dead_code))] fn t() {}')[0], [])

    def test_literal_values_and_trivia_fingerprints(self):
        fp = lambda s: self.scan(s)[0][0]['fingerprint']
        a = '#[test] fn a() { assert_eq!("a b", r#"c"#); }'
        self.assertEqual(fp(a), fp(a.replace('fn a()', 'fn /* trivia */ a ( )')))
        self.assertNotEqual(fp(a), fp(a.replace('a b', 'a  b')))
        self.assertNotEqual(fp(a), fp(a.replace('"c"', '"d"')))

    def test_doctests_all_modes_and_nonrust(self):
        src = '\n'.join('/// ' + s for s in ['```compile_fail', 'bad();', '```', '```ignore', 'x();', '```', '```no_run', 'x();', '```', '```python', 'pass', '```'])
        self.assertEqual(len(self.scan(src)[0]), 3)
        self.assertEqual(len(self.scan('#[doc = "```rust\\nassert!(true);\\n```"] fn a() {}')[0]), 1)

    def test_conditional_doc_literals_count_nested(self):
        src = '#[cfg_attr(feature="x", cfg_attr(any(), doc="```rust\\nassert!(true);\\n```"))] fn a() {}'
        self.assertEqual(len(self.scan(src)[0]), 1)
        _, leads, _, _ = self.scan('#[cfg_attr(any(), doc=include_str!("a.md"))] fn a() {}')
        self.assertTrue(any(x['kind'] == 'doc_source' for x in leads))

    def test_markdown_containers_and_rust_tags(self):
        self.assertEqual(len(self.scan('/// > ```rust\n/// > assert!(true);\n/// > ```')[0]), 1)
        self.assertEqual(len(self.scan('/// ```standalone_crate\n/// assert!(true);\n/// ```')[0]), 1)
        self.assertTrue(self.scan('///         assert!(true);')[1])

    def test_unknown_namespaced_builtin_lookalikes_fail(self):
        _, leads, _, _ = self.scan('#[evil::derive(Foo)] struct A; #[custom::serial] #[test] fn t() {}')
        self.assertEqual(sum(x['kind'] == 'attribute' for x in leads), 2)

    def test_unsupported_generators_and_docs(self):
        _, leads, _, _ = self.scan('#[rstest] fn x() {}\nproptest! { fn y() {} }\n#[doc=include_str!("a.md")] fn z() {}')
        self.assertTrue(any(x['kind'] == 'test_generator' and not x['classifiable'] for x in leads))
        self.assertTrue(any(x['kind'] == 'doc_source' for x in leads))

    def test_macro_definition_and_item_invocation_but_not_expression(self):
        _, leads, _, _ = self.scan('macro_rules! x { () => {} } x!(); fn f() { println!("x"); }')
        self.assertEqual(CounterKinds(leads), {'macro_definition': 1, 'item_macro': 1})

    def test_classification_is_precise_and_cannot_waive_generator(self):
        files = {'x.rs': b'macro_rules! x { () => {} }'}
        inv = gate.inventory(files, EMPTY)
        lead = inv['leads'][0]
        row = dict(lead, conclusion='non_test', rationale='Defines production constants only', author='A', reviewer='B', review_evidence='review/42')
        ledger = dict(EMPTY, classifications=[row])
        self.assertTrue(gate.inventory(files, ledger)['leads'][0]['resolved'])
        self.assertFalse(gate.inventory({'x.rs': files['x.rs'] + b' '}, ledger)['leads'][0]['resolved'])
        gen = gate.inventory({'x.rs': b'proptest! {}'}, EMPTY)['leads'][0]
        row.update(gen)
        self.assertFalse(gate.inventory({'x.rs': b'proptest! {}'}, ledger)['leads'][0]['resolved'])

    def test_generator_inside_test_is_not_hidden(self):
        _, leads, _, _ = self.scan('#[test] fn t() { proptest! {} }')
        self.assertTrue(any(x['kind'] == 'test_generator' for x in leads))

    def test_production_expression_macros_are_not_item_leads(self):
        _, leads, _, _ = self.scan('const X: &str = include_str!("a"); const Y: &[u8] = &[x!(1), x!(2)];')
        self.assertEqual(leads, [])

    def test_unknown_derive_fails_but_known_serde_is_allowed(self):
        _, leads, _, _ = self.scan('#[derive(Debug, serde::Serialize, CustomTestGenerator)] struct A;')
        self.assertEqual([x['kind'] for x in leads], ['derive'])

    def test_malformed_lexical_source_fails_visibly(self):
        self.assertTrue(self.scan('#[test] fn t() { /* unclosed')[2])

    def test_fn_pointer_does_not_hide_later_item_macro(self):
        source = 'pub struct Hooks { pub on_event: fn(u8) } #[cfg(test)] mod tests { some_crate::generate_tests!{alpha} }'
        tests, leads, errors, _ = self.scan(source)
        self.assertEqual(tests, [])
        self.assertEqual(errors, [])
        self.assertEqual(CounterKinds(leads), {'item_macro': 1})

    def test_non_rs_and_expression_module_paths_cannot_be_classified(self):
        for attr in ('path="hidden.txt"', 'path=concat!("hidden", ".rs")', 'cfg_attr(test, path="hidden.txt")'):
            with self.subTest(attr=attr):
                files = {'x.rs': ('#[%s] mod hidden;' % attr).encode()}
                inv = gate.inventory(files, EMPTY)
                lead = next(x for x in inv['leads'] if x['kind'] == 'external_module')
                row = dict(lead, conclusion='non_test', rationale='Opaque certificate', author='A', reviewer='B', review_evidence='review/42')
                checked = gate.inventory(files, dict(EMPTY, classifications=[row]))
                self.assertFalse(next(x for x in checked['leads'] if x['kind'] == 'external_module')['resolved'])
        for literal in ('"hidden.rs"', 'r"hidden.rs"', 'r#"hidden.rs"#'):
            files = {'orphan.rs': ('#[path=%s] mod hidden;' % literal).encode(), 'hidden.rs': b'#[test] fn counted() {}'}
            self.assertEqual(gate.inventory(files, EMPTY)['leads'], [])
            self.assertEqual(gate.inventory(files, EMPTY)['physical_count'], 1)

    def test_include_bounded_forms_require_registered_source(self):
        for expression in ('"hidden.rs"', 'r"hidden.rs"', 'r#"hidden.rs"#',
                           'concat!(env!("CARGO_MANIFEST_DIR"), "/src/hidden.rs")'):
            with self.subTest(expression=expression):
                files = {'pkg/Cargo.toml': b'', 'pkg/src/a.rs': ('include!(%s);' % expression).encode(),
                         'pkg/src/hidden.rs': b'#[test] fn counted() {}'}
                inv = gate.inventory(files, EMPTY)
                self.assertEqual(inv['physical_count'], 1)
                lead = inv['leads'][0]
                self.assertEqual(lead['kind'], 'item_macro')
                self.assertTrue(lead['classifiable'])
                row = dict(lead, conclusion='non_test', rationale='Already counted source', author='A', reviewer='B', review_evidence='review/42')
                ledger = dict(EMPTY, classifications=[row])
                self.assertEqual(gate.check(gate.inventory(files, ledger), files, ledger)[0], [])
                del files['pkg/src/hidden.rs']
                rejected = gate.inventory(files, ledger)
                self.assertTrue(any(x['kind'] == 'external_include' and not x['classifiable'] for x in rejected['leads']))

    def test_unknown_include_expressions_cannot_be_classified(self):
        for expression in ('"hidden.txt"', 'concat!("hidden", ".rs")',
                           'concat!(env!("OUT_DIR"), "/hidden.rs")',
                           'concat!(env!("CARGO_MANIFEST_DIR"), "/hidden", ".rs")',
                           '"/outside.rs"', '"../../outside.rs"'):
            with self.subTest(expression=expression):
                files = {'a.rs': ('include!(%s);' % expression).encode(), 'hidden.rs': b''}
                inv = gate.inventory(files, EMPTY)
                certificates = [dict(x, conclusion='non_test', rationale='Fake', author='A', reviewer='B', review_evidence='review/42') for x in inv['leads']]
                checked = gate.inventory(files, dict(EMPTY, classifications=certificates))
                self.assertTrue(any(x['kind'] == 'external_include' and not x['resolved'] and not x['classifiable'] for x in checked['leads']))

    def test_ordinary_include_str_data_is_not_test_source(self):
        self.assertEqual(self.scan('fn data() -> &\'static str { include_str!("hidden.txt") }')[1], [])


def CounterKinds(rows):
    from collections import Counter
    return Counter(x['kind'] for x in rows)


class RepoTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git('init', '-q')
        self.git('config', 'user.email', 'fixture@example.invalid')
        self.git('config', 'user.name', 'Fixture')
        self.write(gate.ADMISSIONS, json.dumps(EMPTY))
        self.write('a.rs', '#[test] fn kept() { assert_eq!(1, 1); }\n#[test] fn gone() {}\n')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture')
        self.base = self.git('rev-parse', 'HEAD').strip()

    def git(self, *args):
        return subprocess.check_output(['git', *args], cwd=self.root, text=True)

    def write(self, name, text):
        p = self.root / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)

    def run_gate(self, command='check', base=True):
        argv = ['--repo', str(self.root), command]
        if base:
            argv += ['--base', self.base]
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = gate.main(argv)
        return code, json.loads(out.getvalue())

    def test_pure_deletion_and_formatting(self):
        self.write('a.rs', '#[test]\n fn kept ( ) { assert_eq! ( 1 , 1 ); }\n')
        self.assertEqual(self.run_gate()[0], 0)

    def test_untracked_orphan_and_ignored_count(self):
        self.write('orphan.rs', '#[ignore] #[cfg(any())] #[test] fn orphan() {}')
        code, out = self.run_gate()
        self.assertEqual(out['physical_count'], 3)
        self.assertEqual(code, 1)
        self.assertTrue(any('admission' in x for x in out['violations']))

    def test_invalid_explicit_base_fails_even_inventory(self):
        self.base = 'no-such-ref'
        self.assertEqual(self.run_gate('inventory')[0], 2)

    def test_move_multiset_requires_context_not_new_admission(self):
        source = (self.root / 'a.rs').read_text()
        self.write('a.rs', '')
        self.write('other.rs', source)
        code, out = self.run_gate()
        self.assertEqual(code, 1, out)
        self.assertTrue(any('support impact: other.rs' in x for x in out['violations']))
        self.assertFalse(any('admission:' in x for x in out['violations']))
        # Adding a duplicate body is NOT an unchanged move.
        self.write('duplicate.rs', source)
        self.assertEqual(self.run_gate()[0], 1)

    def test_same_file_context_move_requires_impact(self):
        self.write('a.rs', 'mod one { #[test] fn t() {} } mod two {}')
        self.git('add', 'a.rs')
        self.git('commit', '-qm', 'context fixture')
        self.base = self.git('rev-parse', 'HEAD').strip()
        self.write('a.rs', 'mod one {} mod two { #[test] fn t() {} }')
        code, out = self.run_gate()
        self.assertEqual(code, 1)
        self.assertTrue(any('support impact: a.rs' in x for x in out['violations']))
        self.assertFalse(any('admission:' in x for x in out['violations']))

    def test_support_change_requires_hash_bound_review(self):
        path = 'refact-agent/engine/testdata/fixture.json'
        self.write(path, '{}')
        code, out = self.run_gate()
        self.assertEqual(code, 1)
        self.assertTrue(any('support impact: ' + path in x for x in out['violations']))
        ledger = dict(EMPTY, impacts=[dict(path=path, before_sha256=None, after_sha256=gate.sha('{}'), affected_tests=[], no_tests_reason='Not consumed by retained Rust tests', impact='Documentation fixture only', execution_context='No test dependency', author='A', reviewer='B', review_evidence='review/42')])
        self.write(gate.ADMISSIONS, json.dumps(ledger))
        self.assertEqual(self.run_gate()[0], 0)
        self.write(path, '{"changed":true}')
        self.assertEqual(self.run_gate()[0], 1)

    def test_unrelated_tracked_gui_and_untracked_scratch_are_ignored(self):
        for path in ('refact-agent/gui/src/view.tsx', 'plugins/vscode/image.svg', 'docs/unrelated.md'):
            self.write(path, 'before')
        (self.root / 'gui-link').symlink_to('a.rs')
        self.git('add', '.')
        self.git('commit', '-qm', 'unrelated inputs')
        self.base = self.git('rev-parse', 'HEAD').strip()
        for path in ('refact-agent/gui/src/view.tsx', 'plugins/vscode/image.svg', 'docs/unrelated.md', 'scratch.txt'):
            self.write(path, 'after')
        (self.root / 'scratch-link').symlink_to('/missing/outside')
        self.assertEqual(self.run_gate()[0], 0)
        self.assertNotIn('scratch.txt', gate.snapshot(self.root))

    def test_engine_support_extensions_and_policy_are_scoped(self):
        for path in ('refact-agent/engine/fixtures/case.txt', 'refact-agent/engine/assets/a.js',
                     'refact-agent/engine/config/a.yaml', 'refact-agent/engine/snapshots/a.snap',
                     'Cargo.toml', 'tools/dev/check.sh', 'docs/rust-test-policy.md'):
            with self.subTest(path=path):
                self.assertTrue(gate.support_path(path))

    def test_nonrust_module_with_7000_tests_fails_even_with_impact_review(self):
        self.write('a.rs', '#[cfg(test)] #[path="hidden.txt"] mod hidden;')
        self.write('hidden.txt', '\n'.join('#[test] fn hidden_%d() {}' % i for i in range(7000)))
        before = gate.snapshot(self.root, self.base)
        after = gate.snapshot(self.root)
        impact = dict(path='a.rs', before_sha256=gate.sha(before['a.rs']), after_sha256=gate.sha(after['a.rs']), affected_tests=[], no_tests_reason='Claimed no tests', impact='Reviewed', execution_context='Reviewed', author='A', reviewer='B', review_evidence='review/42')
        lead = gate.inventory(after, EMPTY)['leads'][0]
        classification = dict(lead, conclusion='non_test', rationale='Claimed harmless', author='A', reviewer='B', review_evidence='review/42')
        self.write(gate.ADMISSIONS, json.dumps(dict(EMPTY, impacts=[impact], classifications=[classification])))
        code, out = self.run_gate()
        self.assertEqual(code, 1)
        self.assertTrue(any('external_module' in x for x in out['violations']))

    def test_invalid_utf8_identifies_source_path(self):
        (self.root / 'bad.rs').write_bytes(b'\xff')
        code, out = self.run_gate()
        self.assertEqual(code, 2)
        self.assertIn('bad.rs', out['error'])

    def test_external_readme_snapshot_counts_each_usage_and_tracks_change(self):
        path = 'vendor/pkg/src/lib.rs'
        readme = 'vendor/pkg/README.md'
        self.write(path, '#![doc=include_str!("../README.md")]\n'
                         '#[cfg_attr(any(), cfg_attr(any(), doc=include_str!(r"../README.md",)))] pub struct A;')
        self.write(readme, '```rust\nassert_eq!(1, 1);\n```\n'
                           '```\nassert!(true);\n```\n```javascript\nnotRust();\n```')
        files = gate.snapshot(self.root)
        self.assertIn(readme, files)
        inv = gate.inventory(files, EMPTY)
        rows = [t for t in inv['tests'] if t['path'] == path]
        self.assertEqual(len(rows), 4)
        self.assertEqual(len({t['fingerprint'] for t in rows}), 2)
        self.assertTrue(all(t['doc_source'] == readme for t in rows))
        self.assertEqual(inv['leads'], [])
        self.assertEqual(sum(t['kind'] == 'doc_source' for t in gate.scan(path, files[path])[1]), 2)
        self.git('add', '.')
        self.git('commit', '-qm', 'external doc fixture')
        self.base = self.git('rev-parse', 'HEAD').strip()
        self.assertEqual(gate.inventory(gate.snapshot(self.root, self.base), EMPTY)['tests'], inv['tests'])
        self.write(readme, '```rust\nassert_eq!(2, 2);\n```')
        code, out = self.run_gate()
        self.assertEqual(code, 1)
        self.assertTrue(any('admission:' in x for x in out['violations']))
        self.assertTrue(any('support impact: ' + readme in x for x in out['violations']))
        self.write(readme, '    ordinary indented prose\n')
        lead = next(item for item in gate.inventory(gate.snapshot(self.root), EMPTY)['leads']
                    if item['kind'] == 'indented_doc')
        certificate = dict(lead, conclusion='non_test', rationale='Reviewed original prose',
                           author='A', reviewer='B', review_evidence='review/external-doc')
        ledger = dict(EMPTY, classifications=[certificate])
        self.write(readme, '    assert_eq!(1, 2);\n')
        leads = [item for item in gate.inventory(gate.snapshot(self.root), ledger)['leads']
                 if item['kind'] == 'indented_doc']
        self.assertEqual(len(leads), 2)
        self.assertTrue(all(not item['resolved'] for item in leads))
        self.assertTrue(all(readme in item['reason'] for item in leads))

    def test_external_docs_missing_outside_and_symlink_fail_closed(self):
        for target in ('missing.md', '../outside.md', '/outside.md', 'concat!("a", ".md")'):
            expression = target if target.startswith('concat!') else json.dumps(target)
            self.write('a.rs', '#[doc=include_str!(%s)] pub struct A;' % expression)
            code, out = self.run_gate(base=False)
            self.assertEqual(code, 1, out)
            self.assertTrue(any(x['kind'] == 'doc_source' and not x['classifiable'] for x in out['leads']))
        self.write('a.rs', '#[doc=include_str!("linked.md")] pub struct A;')
        (self.root / 'linked.md').symlink_to(self.root / 'a.rs')
        code, out = self.run_gate(base=False)
        self.assertEqual(code, 2)
        self.assertIn('symlink path forbidden', out['error'])

    def test_external_docs_binary_and_size_limit_fail_closed(self):
        self.write('a.rs', '#[doc=include_str!("external.md")] pub struct A;')
        (self.root / 'external.md').write_bytes(b'\xff')
        code, out = self.run_gate(base=False)
        self.assertEqual(code, 1)
        self.assertTrue(any('invalid UTF-8 external documentation' in x['reason'] for x in out['leads']))
        self.write('external.md', 'x' * gate.DOC_SOURCE_LIMIT)
        self.assertEqual(self.run_gate(base=False)[0], 0)
        self.write('external.md', 'x' * (gate.DOC_SOURCE_LIMIT + 1))
        code, out = self.run_gate(base=False)
        self.assertEqual(code, 2)
        self.assertIn('exceeds 1 MiB', out['error'])
        self.git('add', '.')
        self.git('commit', '-qm', 'oversize external doc fixture')
        self.base = self.git('rev-parse', 'HEAD').strip()
        self.write('external.md', '')
        code, out = self.run_gate()
        self.assertEqual(code, 2)
        self.assertIn('exceeds 1 MiB', out['error'])

    def test_7000_external_include_and_doc_tests_reject_fake_certificates(self):
        for source, payload, kind in (
                ('include!("hidden.txt");', '\n'.join('#[test] fn hidden_%d() {}' % i for i in range(7000)), 'external_include'),
                ('#[doc=include_str!(concat!("hidden", ".txt"))] pub struct Documented;', '```rust\nassert!(true);\n```\n' * 7000, 'doc_source')):
            with self.subTest(kind=kind):
                self.write('a.rs', source)
                self.write('hidden.txt', payload)
                files = gate.snapshot(self.root)
                self.assertNotIn('hidden.txt', files)
                inv = gate.inventory(files, EMPTY)
                certificates = [dict(x, conclusion='non_test', rationale='Fake source-only review', author='A', reviewer='B', review_evidence='review/42') for x in inv['leads']]
                self.write(gate.ADMISSIONS, json.dumps(dict(EMPTY, classifications=certificates)))
                code, out = self.run_gate(base=False)
                self.assertEqual(code, 1)
                self.assertEqual(out['physical_count'], 0)
                self.assertTrue(any(x['kind'] == kind and not x['resolved'] and not x['classifiable'] for x in out['leads']))

    def test_missing_and_ignored_rs_include_and_module_targets_fail(self):
        self.write('.gitignore', 'hidden.rs\n')
        self.write('hidden.rs', '#[test] fn hidden() {}')
        for source, kind in (('include!("hidden.rs");', 'external_include'),
                             ('#[path="hidden.rs"] mod hidden;', 'external_module')):
            for present in (True, False):
                with self.subTest(source=source, present=present):
                    if present:
                        self.write('hidden.rs', '#[test] fn hidden() {}')
                    else:
                        (self.root / 'hidden.rs').unlink()
                    self.write('a.rs', source)
                    files = gate.snapshot(self.root)
                    self.assertNotIn('hidden.rs', files)
                    inv = gate.inventory(files, EMPTY)
                    certificates = [dict(x, conclusion='non_test', rationale='Fake', author='A', reviewer='B', review_evidence='review/42') for x in inv['leads']]
                    self.write(gate.ADMISSIONS, json.dumps(dict(EMPTY, classifications=certificates)))
                    code, out = self.run_gate(base=False)
                    self.assertEqual(code, 1)
                    self.assertTrue(any(x['kind'] == kind and not x['resolved'] for x in out['leads']))

    def test_nonrust_engine_support_symlink_is_rejected(self):
        path = self.root / 'refact-agent/engine/testdata/linked.txt'
        path.parent.mkdir(parents=True)
        path.symlink_to(self.root / 'a.rs')
        code, out = self.run_gate()
        self.assertEqual(code, 2)
        self.assertIn('symlink path forbidden', out['error'])
        self.assertIn('linked.txt', out['error'])

    def test_common_ledger_schema_reports_current_and_base_paths(self):
        malformed = json.dumps(dict(EMPTY, tests={}))
        self.write(gate.ADMISSIONS, malformed)
        code, out = self.run_gate()
        self.assertEqual(code, 2)
        self.assertIn(gate.ADMISSIONS, out['error'])
        self.git('add', gate.ADMISSIONS)
        self.git('commit', '-qm', 'malformed base ledger')
        self.base = self.git('rev-parse', 'HEAD').strip()
        self.write(gate.ADMISSIONS, json.dumps(EMPTY))
        code, out = self.run_gate()
        self.assertEqual(code, 2)
        self.assertIn('base:' + gate.ADMISSIONS, out['error'])

    def test_git_ignored_rust_is_not_claimed_tracked(self):
        self.write('.gitignore', 'ignored.rs\n')
        self.write('ignored.rs', '#[test] fn ignored() {}')
        self.assertNotIn('ignored.rs', gate.snapshot(self.root))
        self.assertEqual(self.run_gate()[1]['physical_count'], 2)

    def admission(self, fingerprint, nearest, critical=False):
        row = {k: 'Concrete reviewed evidence' for k in ('failure_mode', 'importance', 'oracle', 'prerequisites', 'shipping_implementation', 'determinism', 'layer', 'why_cheapest', 'criticality_reason', 'removed_coverage')}
        row.update(fingerprint=fingerprint, occurrences=1, meaningful_cases=1, critical=critical, nearest_retained={'fingerprint': nearest, 'why_insufficient': 'Does not exercise the failed guard'}, author='A', reviewer='B', review_evidence='review/42', witness={'kind': 'counterfactual', 'explanation': 'Missing guard returns forbidden identity', 'evidence': 'review/42'})
        return row

    def test_material_change_admission_and_distinct_review(self):
        self.write('a.rs', '#[test] fn kept() { assert_eq!(2, 2); }\n#[test] fn gone() {}')
        inv = gate.inventory(gate.snapshot(self.root), EMPTY)
        row = self.admission(inv['tests'][0]['fingerprint'], inv['tests'][1]['fingerprint'])
        self.write(gate.ADMISSIONS, json.dumps(dict(EMPTY, tests=[row])))
        self.assertEqual(self.run_gate()[0], 0)
        row['reviewer'] = 'a'
        self.write(gate.ADMISSIONS, json.dumps(dict(EMPTY, tests=[row])))
        self.assertEqual(self.run_gate()[0], 1)

    def test_critical_witness_requires_structured_execution(self):
        row = self.admission('a', 'b', True)
        self.assertFalse(gate.valid_test(row, {'b': 1}))
        row['witness'] = {'kind': 'executed', 'failing': {'command': ['cargo', 'test', 'guard'], 'exit_code': 101, 'evidence': 'logs/fail'}, 'passing': {'command': ['cargo', 'test', 'guard'], 'exit_code': 0, 'evidence': 'logs/pass'}, 'intended_diagnostic': 'assertion failed: forbidden identity absent', 'production_target': 'shipping::guard', 'before_fix': {'reference': 'mutant patch', 'fingerprint': 'a' * 64}}
        self.assertTrue(gate.valid_test(row, {'b': 1}))
        row['witness']['failing']['exit_code'] = 0
        self.assertFalse(gate.valid_test(row, {'b': 1}))

    def test_nearest_cannot_be_self_or_mutually_new_tests(self):
        self.write('a.rs', '#[test] fn added_a() {} #[test] fn added_b() {}')
        tests = gate.inventory(gate.snapshot(self.root), EMPTY)['tests']
        a, b = [t['fingerprint'] for t in tests]
        self.assertFalse(gate.valid_test(self.admission(a, a), {a: 1, b: 1}))
        self.write(gate.ADMISSIONS, json.dumps(dict(EMPTY, tests=[self.admission(a, b), self.admission(b, a)])))
        code, out = self.run_gate()
        self.assertEqual(code, 1)
        self.assertEqual(sum('missing/invalid admission:' in x for x in out['violations']), 2)

    def test_duplicate_records_report_once_and_count_once(self):
        head = gate.inventory(gate.snapshot(self.root), EMPTY)
        a, b = [t['fingerprint'] for t in head['tests']]
        row = dict(self.admission(a, b), meaningful_cases=2)
        ledger = dict(EMPTY, tests=[row, row])
        errors, meaningful = gate.check(head, {}, ledger)
        self.assertEqual(meaningful, 3)
        self.assertEqual(sum('duplicate admission' in x for x in errors), 1)
        errors, meaningful = gate.check(head, {}, ledger, head, {}, EMPTY)
        self.assertEqual(meaningful, 3)
        self.assertEqual(sum('duplicate admission' in x for x in errors), 1)

    def test_hard_budget_and_meaningful_workload(self):
        head = gate.inventory({'x.rs': b'#[test] fn a() {} #[test] fn b() {}'}, EMPTY)
        row = self.admission(head['tests'][0]['fingerprint'], head['tests'][1]['fingerprint'])
        row['meaningful_cases'] = gate.LIMIT - 1
        errors, meaningful = gate.check(head, {}, dict(EMPTY, tests=[row]))
        self.assertEqual(meaningful, gate.LIMIT)
        self.assertEqual(errors, [])
        row['meaningful_cases'] = gate.LIMIT
        errors, meaningful = gate.check(head, {}, dict(EMPTY, tests=[row]))
        self.assertEqual(meaningful, gate.LIMIT + 1)
        self.assertTrue(any('hard budget' in x for x in errors))
        head['physical_count'] = gate.LIMIT
        self.assertEqual(gate.check(head, {}, EMPTY)[0], [])
        head['physical_count'] = gate.LIMIT + 1
        self.assertTrue(any('hard budget' in x for x in gate.check(head, {}, EMPTY)[0]))

    def test_base_symlink_is_rejected(self):
        (self.root / 'link.rs').symlink_to('a.rs')
        self.git('add', 'link.rs')
        self.git('commit', '-qm', 'symlink fixture')
        self.base = self.git('rev-parse', 'HEAD').strip()
        (self.root / 'link.rs').unlink()
        code, out = self.run_gate('inventory')
        self.assertEqual(code, 2)
        self.assertIn('nonregular base', out['error'])

    def test_unchanged_body_helper_edit_requires_impact(self):
        self.write('a.rs', (self.root / 'a.rs').read_text() + '\nfn helper() -> u32 { 42 }')
        code, out = self.run_gate()
        self.assertEqual(code, 1)
        self.assertTrue(any('support impact: a.rs' in x for x in out['violations']))
        self.assertFalse(any('admission:' in x for x in out['violations']))

    def test_base_workload_record_cannot_be_deleted_or_reduced(self):
        inv = gate.inventory(gate.snapshot(self.root), EMPTY)
        row = self.admission(inv['tests'][0]['fingerprint'], inv['tests'][1]['fingerprint'])
        row['meaningful_cases'] = gate.LIMIT + 1
        self.write(gate.ADMISSIONS, json.dumps(dict(EMPTY, tests=[row])))
        self.git('add', gate.ADMISSIONS)
        self.git('commit', '-qm', 'known aggregate workload')
        self.base = self.git('rev-parse', 'HEAD').strip()
        for records in ([], [dict(row, meaningful_cases=1)]):
            self.write(gate.ADMISSIONS, json.dumps(dict(EMPTY, tests=records)))
            code, out = self.run_gate()
            self.assertEqual(code, 1)
            self.assertGreater(out['meaningful_count'], gate.LIMIT)
            self.assertTrue(any('accounting removed/reduced' in x for x in out['violations']))

    def test_duplicate_json_keys_fail(self):
        self.write(gate.ADMISSIONS, '{"version":1,"tests":[],"tests":[],"impacts":[],"classifications":[]}')
        self.assertEqual(self.run_gate()[0], 2)

    def test_malformed_admission_fails_without_traceback(self):
        self.write(gate.ADMISSIONS, json.dumps(dict(EMPTY, tests=[{'fingerprint': []}])))
        self.assertNotEqual(self.run_gate()[0], 0)

    def test_symlinks_and_outside_admissions_fail(self):
        (self.root / 'linked.rs').symlink_to('a.rs')
        self.assertEqual(self.run_gate()[0], 2)
        (self.root / 'linked.rs').unlink()
        with contextlib.redirect_stdout(io.StringIO()):
            code = gate.main(['--repo', str(self.root), 'check', '--admissions', '/etc/passwd'])
        self.assertEqual(code, 2)


if __name__ == '__main__':
    unittest.main()

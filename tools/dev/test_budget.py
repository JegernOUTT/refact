#!/usr/bin/env python3
"""Conservative Rust test budget/admission gate (Python 3.10+, stdlib + git).

Usage: test_budget.py inventory [--base REF] [--admissions PATH]
       test_budget.py check [--base REF] [--admissions PATH]

Inventory is lexical, not a Rust compiler. It includes orphan/cfg/ignored/new .rs
files. Unresolved generator/doc leads make check fail; classifiers may ONLY
certify a precisely hashed lead as non-test, never assign a count. No ceiling
override exists. Without --base only inventory/discovery/budget are enforced.

Admissions JSON version 1 has lists: tests, impacts, classifications. Test rows:
fingerprint, occurrences (positive int), failure_mode, importance,
nearest_retained {fingerprint, why_insufficient}, oracle, prerequisites,
shipping_implementation, determinism, layer, why_cheapest, meaningful_cases,
critical (bool), criticality_reason, removed_coverage, witness, author, reviewer,
review_evidence. Prose fields are nonempty strings. Ordinary witness:
{kind: counterfactual, explanation, evidence}; executed witness:
{kind: executed, failing: {command: [str], exit_code: nonzero int, evidence},
passing: {command: [str], exit_code: 0, evidence}, intended_diagnostic,
production_target, before_fix: {reference, fingerprint: SHA256}}.
Critical rows require executed witnesses. Meaningful cases are PER occurrence.

Impact rows: path, before_sha256, after_sha256 (null for absent),
affected_tests ([current fingerprints], or [] with no_tests_reason), impact,
execution_context, author, reviewer, review_evidence. Hashes are raw full-file
hashes, not residue hashes. Changed Rust/build/test-support residue paths require
impact review. Even moves require impact/context review. Admissions file itself
is excluded to avoid a self-hash cycle; review protections must secure it.

Classification rows: path, kind, source_sha256, lead_sha256, conclusion:
non_test, rationale, author, reviewer, review_evidence. Rows are exact bounded
source/lead certificates; unresolved test generators cannot be waived. Inventory
prints all leads and their hashes for review. This gate cannot authenticate prose,
reviewer independence, actual witness execution, or all meaningful workload
packing. Repository review and real builds remain mandatory.
"""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import posixpath
import re
import subprocess
import sys

LIMIT = 6400
ADMISSIONS = 'tools/dev/test_admissions.json'
ATTRIBUTE_GENERATORS = {'rstest', 'test_case', 'test_matrix', 'quickcheck', 'proptest', 'parameterized', 'wasm_bindgen_test'}
MACRO_GENERATORS = {'proptest', 'quickcheck', 'test_case', 'test_cases', 'parameterized', 'test_suite'}
LIMITATIONS = [
    'Lexical inventory: no cfg/reachability evaluation or arbitrary macro expansion.',
    'Physical definitions are a lower bound; semantic review must detect packed workloads.',
    'Evidence structure does not authenticate execution, semantic truth, or reviewer independence.',
    'Without --base, admission and support-impact comparison is not performed.',
    'Admissions ledger requires repository review protections (excluded from self-hashing).',
]


def sha(value):
    return hashlib.sha256(value.encode() if isinstance(value, str) else value).hexdigest()


def git(root, *args):
    p = subprocess.run(['git', '-c', 'core.fsmonitor=false', '-c', 'core.hooksPath=/dev/null', *args],
                       cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if p.returncode:
        raise ValueError('git ' + ' '.join(args) + ': ' + p.stderr.decode(errors='replace').strip())
    return p.stdout


def safe_file(root, file):
    try:
        relative = file.absolute().relative_to(root)
    except ValueError:
        raise ValueError('path outside repository: ' + str(file))
    cursor = root
    for part in relative.parts:
        if part == '..':
            raise ValueError('parent path forbidden: ' + str(file))
        cursor = cursor / part
        if cursor.is_symlink():
            raise ValueError('symlink path forbidden: ' + str(file))
    if not file.is_file():
        raise ValueError('nonregular file forbidden: ' + str(file))


def support_path(path):
    """Rust test inputs, not unrelated GUI/plugin/docs or non-Rust scratch.

    All engine inputs are included regardless of extension (fixtures, assets,
    snapshots, scripts, etc.). Ignored files are not claimed as tracked inputs.
    """
    p = Path(path)
    return (path.endswith('.rs') or path.startswith(('refact-agent/engine/', 'tools/dev/', '.cargo/'))
            or p.name in {'Cargo.toml', 'Cargo.lock', 'rust-toolchain', 'rust-toolchain.toml'}
            or path in {'docs/rust-test-policy.md',
                        '.github/PULL_REQUEST_TEMPLATE.md', '.github/workflows/rust_test_budget.yml'}
            or (path.startswith('.github/workflows/') and ('engine' in p.name or 'rust' in p.name)))


def snapshot(root, base=None):
    if base is not None:
        # Resolve before listing: an invalid explicitly requested base never falls back.
        ref = git(root, 'rev-parse', '--verify', base + '^{commit}').decode().strip()
        entries = git(root, 'ls-tree', '-r', '-z', ref).decode().split('\0')
        result = {}
        for entry in filter(None, entries):
            meta, path = entry.split('\t', 1)
            mode, kind, oid = meta.split()
            if not support_path(path):
                continue
            if kind != 'blob' or mode not in ('100644', '100755'):
                raise ValueError('nonregular base object forbidden: ' + path)
            result[path] = git(root, 'cat-file', 'blob', oid)
        return result
    paths = git(root, 'ls-files', '-z', '--cached', '--others', '--exclude-standard').decode().split('\0')
    result = {}
    for p in sorted(set(paths) - {''}):
        if not support_path(p):
            continue
        file = root / p
        if file.exists() or file.is_symlink():
            safe_file(root, file)
            result[p] = file.read_bytes()
    return result


def lex(src):
    """Return tokens (text,start,end), documentation spans and lexical errors.

    Literal tokens retain their EXACT spelling, never a masked literal hash.
    Nested comments are skipped; lifetimes are not mistaken for char literals.
    """
    ts, docs, errors = [], [], []
    i, n = 0, len(src)
    raw_re = re.compile(r'(?:br|cr|r)(#*)"')
    normal_re = re.compile(r'(?:b|c)?"')
    char_re = re.compile(r"(?:b)?'(?:[^'\\\n]|\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|[^\n]))'")
    token_re = re.compile(r'[A-Za-z_][A-Za-z_0-9]*|::|[^\s]')
    while i < n:
        a = i
        if src[i].isspace():
            i += 1
            continue
        if src.startswith('//', i):
            i = src.find('\n', i)
            if i < 0:
                i = n
            if src.startswith(('///', '//!'), a) and not src.startswith('////', a):
                docs.append((a, i))
            continue
        if src.startswith('/*', i):
            depth, i = 1, i + 2
            while i < n and depth:
                if src.startswith('/*', i):
                    depth, i = depth + 1, i + 2
                elif src.startswith('*/', i):
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
            if depth:
                errors.append((a, 'unterminated comment'))
            if src.startswith(('/**', '/*!'), a) and not src.startswith('/***', a):
                docs.append((a, i))
            continue
        raw = raw_re.match(src, i) if src[i] in 'brc' else None
        normal = normal_re.match(src, i) if src[i] in 'bc"' else None
        char = char_re.match(src, i)
        if raw:
            end = '"' + raw[1]
            j = src.find(end, i + len(raw[0]))
            i = n if j < 0 else j + len(end)
            if j < 0:
                errors.append((a, 'unterminated raw string'))
        elif normal:
            i += len(normal[0])
            closed = False
            while i < n:
                if src[i] == '\\':
                    i += 2
                elif src[i] == '"':
                    i += 1
                    closed = True
                    break
                else:
                    i += 1
            if not closed:
                errors.append((a, 'unterminated string'))
        elif char:
            i += len(char[0])
        else:
            m = token_re.match(src, i)
            i += len(m[0])
        ts.append((src[a:i], a, i))
    return ts, docs, errors


def canonical(src):
    return [t[0] for t in lex(src)[0]]


def paired(ts):
    stack, pairs, errors = [], {}, []
    for i, (t, a, _) in enumerate(ts):
        if t in ('(', '[', '{'):
            stack.append(i)
        elif t in (')', ']', '}'):
            if not stack or ts[stack[-1]][0] != {')': '(', ']': '[', '}': '{'}[t]:
                errors.append((a, 'unmatched delimiter'))
            else:
                j = stack.pop()
                pairs[i], pairs[j] = j, i
    errors.extend((ts[i][1], 'unclosed delimiter') for i in stack)
    return pairs, errors


def meta_items(values):
    """Expand cfg_attr meta syntax only; never inspect literal contents as syntax."""
    if not values or values[0] != 'cfg_attr':
        return [values] if values else []
    if len(values) < 3 or values[1] != '(' or values[-1] != ')':
        return []
    chunks, current, depth = [], [], 0
    for token in values[2:-1]:
        if token == ',' and depth == 0:
            chunks.append(current)
            current = []
            continue
        current.append(token)
        if token in ('(', '[', '{'):
            depth += 1
        elif token in (')', ']', '}'):
            depth -= 1
    chunks.append(current)
    return [item for chunk in chunks[1:] for item in meta_items(chunk)]


def meta_paths(values):
    paths = []
    for item in meta_items(values):
        path = []
        for token in item:
            if token in ('(', '='):
                break
            path.append(token)
        paths.append(''.join(path))
    return paths


def source_literal(token):
    """Decode only a Rust string literal, never an expression or environment."""
    raw = re.fullmatch(r'r(\#*)"(.*)"\1', token, re.S)
    if raw:
        return raw[2]
    try:
        value = json.loads(token)
        return value if isinstance(value, str) else None
    except (ValueError, TypeError):
        return None


def included_source(path, values, files):
    """Resolve two bounded include forms against the independently scanned registry.

    CARGO_MANIFEST_DIR means the nearest registered Cargo.toml, not the process
    environment. No macro evaluation, filesystem reads, or candidate-name search.
    """
    target = source_literal(values[0]) if len(values) == 1 else None
    parent = Path(path).parent
    if (len(values) == 11 and values[:5] == ['concat', '!', '(', 'env', '!']
            and values[5:9] == ['(', '"CARGO_MANIFEST_DIR"', ')', ',']
            and values[-1] == ')'):
        suffix = source_literal(values[9])
        if not suffix or not suffix.startswith('/') or suffix.startswith('//'):
            return None
        manifest = next((p for p in (parent, *parent.parents)
                         if str(p / 'Cargo.toml') in (files or {})), None)
        if manifest is None:
            return None
        parent, target = manifest, suffix[1:]
    if not target or not target.endswith('.rs') or Path(target).is_absolute() or '\\' in target:
        return None
    resolved = posixpath.normpath(str(parent / target))
    if resolved.startswith('../') or resolved == '..' or resolved not in (files or {}):
        return None
    return resolved


def scan(path, data, files=None):
    try:
        src = data.decode('utf-8')
    except UnicodeError as exc:
        raise ValueError('invalid UTF-8 source: ' + path + ': ' + str(exc)) from exc
    ts, docs, errors = lex(src)
    pair, pe = paired(ts)
    errors += pe
    tests, leads, spans, attrs, functions = [], [], [], {}, []
    source_hash = sha(data)
    contexts = []
    for op, en in sorted(pair.items()):
        if op < en and ts[op][0] == '{':
            start = op - 1
            while start >= 0 and ts[start][0] not in (';', '{', '}'):
                start -= 1
            contexts.append((ts[op][1], ts[en][2], [x[0] for x in ts[start + 1:op]]))

    def lead(kind, a, b, reason, resolvable=True):
        leads.append(dict(path=path, kind=kind, line=src.count('\n', 0, a) + 1,
                          source_sha256=source_hash, lead_sha256=sha(src[a:b]),
                          reason=reason, classifiable=resolvable))

    def test(kind, a, b, name, body=None):
        text = src[a:b] if body is None else body
        fingerprint = sha(json.dumps([kind, canonical(text)], ensure_ascii=False))
        enclosing = [header for start, end, header in contexts if start < a < end]
        tests.append(dict(path=path, line=src.count('\n', 0, a) + 1,
                          name=name, kind=kind, fingerprint=fingerprint,
                          context_sha256=sha(json.dumps(enclosing))))
        spans.append((a, b))

    for i, (t, a, _) in enumerate(ts):
        op = i + 2 if i + 1 < len(ts) and ts[i + 1][0] == '!' else i + 1
        if t == '#' and op < len(ts) and ts[op][0] == '[' and op in pair:
            en = pair[op]
            attrs[i] = (en, [x[0] for x in ts[op + 1:en]])
        if t != 'fn' or i + 1 >= len(ts) or not re.fullmatch(r'[A-Za-z_][A-Za-z_0-9]*', ts[i + 1][0]):
            continue
        j = i + 2
        while j < len(ts) and ts[j][0] not in ('{', ';', '}'):
            j = pair[j] + 1 if j in pair and ts[j][0] in ('(', '[') else j + 1
        if j >= len(ts) or ts[j][0] != '{' or j not in pair:
            continue
        functions.append((i, pair[j]))
        k = i - 1
        while k >= 0 and ts[k][0] in ('pub', 'async', 'unsafe', 'extern', 'const', 'default'):
            k -= 1
        if k >= 0 and ts[k][0] == ')' and k in pair and pair[k] > 0 and ts[pair[k] - 1][0] == 'pub':
            k = pair[k] - 2
        attached = []
        while k >= 0 and ts[k][0] == ']' and k in pair:
            ai = pair[k] - 1
            if ai not in attrs:
                break
            attached.append(ai)
            k = ai - 1
        if not attached:
            continue
        # Match test path at a meta-item boundary, including nested cfg_attr.
        values = [attrs[x][1] for x in attached]
        paths = [p for v in values for p in meta_paths(v)]
        has_test = any(p.split('::')[-1] == 'test' for p in paths)
        generated = any(p.split('::')[-1] in ATTRIBUTE_GENERATORS for p in paths)
        if generated:
            lead('test_generator', ts[min(attached)][1], ts[pair[j]][2], 'unsupported parameterized/custom test attribute', False)
        if has_test or generated:
            test('function', ts[min(attached)][1], ts[pair[j]][2], ts[i + 1][0])

    # Unattached test attributes and unknown item attributes must not silently vanish.
    known_attrs = {'serial', 'parallel', 'skip', 'allow', 'expect', 'warn', 'deny', 'forbid', 'cfg', 'cfg_attr', 'derive', 'doc', 'test', 'ignore', 'should_panic', 'inline', 'cold', 'must_use', 'repr', 'path', 'no_mangle', 'export_name', 'link', 'link_name', 'link_section', 'macro_export', 'macro_use', 'non_exhaustive', 'deprecated', 'track_caller', 'automatically_derived', 'used', 'global_allocator', 'recursion_limit', 'type_length_limit', 'feature', 'no_std', 'no_main', 'windows_subsystem', 'async_trait', 'serde', 'instrument', 'error', 'source', 'from', 'transparent', 'command', 'arg', 'value', 'group', 'builder', 'default', 'pin', 'prost', 'strum', 'serde_as', 'async_recursion'}
    for ai, (en, v) in attrs.items():
        a, b = ts[ai][1], ts[en][2]
        inside_test = any(x <= a and b <= y for x, y in spans)
        text = ''.join(v)
        if any(p.split('::')[-1] == 'test' for p in meta_paths(v)) and not inside_test:
            lead('unattached_test', a, b, 'test attribute not attached to recognized function', False)
        known_derives = {'Debug', 'Clone', 'Copy', 'Default', 'PartialEq', 'Eq', 'PartialOrd', 'Ord', 'Hash', 'Serialize', 'Deserialize', 'serde::Serialize', 'serde::Deserialize', 'Error', 'thiserror::Error', 'EnumString', 'Display', 'EnumIter', 'AsRefStr', 'IntoStaticStr', 'VariantNames', 'EnumCount', 'ValueEnum', 'Parser', 'Args', 'Subcommand', 'RustEmbed', 'Builder', 'TypedBuilder', 'JsonSchema', 'schemars::JsonSchema', 'TS', 'strum::Display', 'strum::EnumString', 'strum::EnumIter', 'strum::AsRefStr', 'strum::EnumCount', 'strum::VariantNames', 'clap::Parser', 'clap::Args', 'clap::Subcommand', 'clap::ValueEnum', 'rust_embed::RustEmbed'}
        for item in meta_items(v):
            if item and item[0] == 'derive':
                names = ''.join(item[2:-1]).split(',')
                for name in filter(None, names):
                    if name not in known_derives:
                        lead('derive', a, b, 'unknown derive may generate tests: ' + name)
        for attr_path in meta_paths(v):
            if attr_path not in known_attrs and attr_path not in {'tokio::test', 'test_log::test', 'async_std::test', 'serial_test::serial', 'serial_test::parallel', 'rustfmt::skip', 'tower_lsp::async_trait', 'async_trait::async_trait', 'tracing::instrument'}:
                lead('attribute', a, b, 'unknown attribute may generate tests: ' + attr_path)
        for item in meta_items(v):
            if item and item[0] == 'path':
                # .rs targets are inventoried independently, including orphan/cfg
                # files. Other extensions and expressions cannot be certified away.
                target = included_source(path, item[2:], files) if len(item) == 3 and item[1] == '=' else None
                if target is None:
                    lead('external_module', a, b, 'module path is not a registered .rs source', False)
            if len(item) < 2 or item[:2] != ['doc', '=']:
                continue
            if len(item) == 3:
                try:
                    literal = item[2]
                    raw = re.fullmatch(r'r(#+)?"(.*)"\1', literal, re.S) if literal.startswith('r#') else None
                    content = raw[2] if raw else literal[2:-1] if literal.startswith('r"') else json.loads(literal)
                    doc_tests(content, a, b, test, lead)
                    continue
                except (ValueError, TypeError):
                    pass
            lead('doc_source', a, b, 'unsupported doc expression/external documentation cannot be certified', False)

    groups = []
    for a, b in docs:
        if groups and not src[groups[-1][1]:a].strip():
            groups[-1] = (groups[-1][0], b)
        else:
            groups.append((a, b))
    for a, b in groups:
        content = '\n'.join(re.sub(r'^\s*(?:///|//!|/\*\*|/\*!|\*/|\*) ?', '', line) for line in src[a:b].splitlines())
        doc_tests(content, a, b, test, lead)

    definition_spans = [(k, pair[k + 3]) for k in range(len(ts) - 3)
                        if ts[k][0] == 'macro_rules' and k + 3 in pair]
    for i, (t, a, _) in enumerate(ts):
        definition = t == 'macro_rules' and i + 3 < len(ts) and ts[i + 1][0] == '!'
        invocation = t == '!' and i > 0 and i + 1 < len(ts) and ts[i + 1][0] in ('(', '[', '{')
        if not (definition or invocation):
            continue
        op = i + 3 if definition else i + 1
        if op not in pair:
            continue
        b = ts[pair[op]][2]
        name = ts[i + 2][0] if definition else ts[i - 1][0]
        in_definition = any(k < i < en for k, en in definition_spans)
        if not definition and name == 'include' and not in_definition:
            values = [x[0] for x in ts[op + 1:pair[op]]]
            if included_source(path, values, files) is None:
                lead('external_include', a, b, 'include target is not a bounded registered .rs source', False)
                continue
        if any(x <= a and b <= y for x, y in spans) and name not in MACRO_GENERATORS:
            continue
        # Macro_rules body and each item-position invocation are bounded leads.
        # Expression macros in ordinary function bodies cannot add module tests,
        # but known test generators remain rejected even there.
        generator = name in MACRO_GENERATORS
        inside_fn = any(fi < i < en for fi, en in functions)
        if in_definition:
            continue
        if generator:
            lead('test_generator', a, b, 'unsupported macro-generated workload', False)
        elif definition or not inside_fn:
            # Item macros start an item, not an initializer/array/argument.
            # Walk to the current item boundary; skip balanced prior attributes.
            k = i - 2 if not definition else i - 1
            while k >= 0 and ts[k][0] not in (';', '{', '}', '='):
                if ts[k][0] in (')', ']') and k in pair:
                    k = pair[k] - 1
                elif ts[k][0] in ('(', '['):
                    break
                else:
                    k -= 1
            expression = k >= 0 and ts[k][0] in ('=', '(', '[')
            if definition or not expression:
                lead('macro_definition' if definition else 'item_macro', a, b, 'macro expansion may define tests')
    residue = list(src)
    for a, b in spans:
        residue[a:b] = ' ' * (b - a)
    return tests, leads, [dict(path=path, line=src.count('\n', 0, a) + 1, reason=e) for a, e in errors], canonical(''.join(residue))


def doc_tests(content, a, b, test, lead):
    opened = None
    lines = content.splitlines()
    for i, line in enumerate(lines):
        # Rustdoc understands Markdown containers; normalize blockquotes for
        # fences and flag list-contained/indented forms we cannot count exactly.
        line = re.sub(r'^(?:\s{0,3}> ?)+', '', line)
        if re.match(r'^\s*(?:[-+*]|\d+[.)])\s+.*(?:`{3,}|~{3,})', line):
            lead('doc_container', a, b, 'list-contained code fence requires classification')
        m = re.match(r'^\s{0,3}(`{3,}|~{3,})(.*)$', line)
        if m:
            fence, info = m.groups()
            if opened is None:
                tags = [x for x in re.split(r'[,\s]+', info.strip()) if x]
                rust = not tags or any(x in {'rust', 'no_run', 'compile_fail', 'ignore', 'should_panic', 'standalone_crate', 'test_harness'} or x.startswith(('edition20', 'ignore-')) for x in tags)
                opened = (i, fence, rust)
            elif fence[0] == opened[1][0] and len(fence) >= len(opened[1]) and not info.strip():
                start, _, rust = opened
                if rust:
                    test('doctest', a, b, 'doctest', '\n'.join(lines[start:i + 1]))
                opened = None
        elif opened is None and re.match(r'^(?: {4,}|\t+)\S', line):
            lead('indented_doc', a, b, 'indented documentation code requires classification')
    if opened:
        lead('doc_fence', a, b, 'unclosed documentation fence', False)


def inventory(files, ledger):
    rows, leads, errors, residues = [], [], [], {}
    for path, data in sorted(files.items()):
        if path.endswith('.rs'):
            tests, pending, errs, residue = scan(path, data, files)
            rows.extend(tests)
            leads.extend(pending)
            errors.extend(errs)
            residues[path] = residue
        elif path != ADMISSIONS and support_path(path):
            residues[path] = sha(data)
    for lead in leads:
        lead['resolved'] = False
        for record in ledger.get('classifications', []):
            if all(record.get(k) == lead[k] for k in ('path', 'kind', 'source_sha256', 'lead_sha256')):
                if lead['classifiable'] and record.get('conclusion') == 'non_test' and reviewed(record) and nonempty(record.get('rationale')):
                    lead['resolved'] = True
    return dict(physical_count=len(rows), tests=rows, leads=leads, errors=errors,
                rust_files=sum(p.endswith('.rs') for p in files), residues=residues)


def nonempty(v):
    return isinstance(v, str) and bool(v.strip())


def reviewed(row):
    return all(nonempty(row.get(k)) for k in ('author', 'reviewer', 'review_evidence')) and row['author'].strip().casefold() != row['reviewer'].strip().casefold()


def integer(v, minimum=1):
    return type(v) is int and v >= minimum


def executed(w):
    if not isinstance(w, dict) or w.get('kind') != 'executed':
        return False
    for key in ('failing', 'passing'):
        run = w.get(key, {})
        if not isinstance(run, dict) or not isinstance(run.get('command'), list) or not run['command'] or not all(nonempty(x) for x in run['command']) or not nonempty(run.get('evidence')) or type(run.get('exit_code')) is not int:
            return False
        if (key == 'passing' and run['exit_code'] != 0) or (key == 'failing' and run['exit_code'] == 0):
            return False
    before = w.get('before_fix', {})
    return all(nonempty(w.get(k)) for k in ('intended_diagnostic', 'production_target')) and isinstance(before, dict) and nonempty(before.get('reference')) and bool(re.fullmatch('[0-9a-f]{64}', str(before.get('fingerprint', ''))))


def valid_test(row, retained):
    fields = ('failure_mode', 'importance', 'oracle', 'prerequisites', 'shipping_implementation', 'determinism', 'layer', 'why_cheapest', 'criticality_reason', 'removed_coverage')
    nearest = row.get('nearest_retained', {})
    if not reviewed(row) or not all(nonempty(row.get(k)) for k in fields) or not integer(row.get('occurrences')) or not integer(row.get('meaningful_cases')) or type(row.get('critical')) is not bool:
        return False
    if not isinstance(nearest, dict) or nearest.get('fingerprint') == row.get('fingerprint') or nearest.get('fingerprint') not in retained or not nonempty(nearest.get('why_insufficient')):
        return False
    w = row.get('witness', {})
    return executed(w) or (not row['critical'] and isinstance(w, dict) and w.get('kind') == 'counterfactual' and all(nonempty(w.get(k)) for k in ('explanation', 'evidence')))


def check(head, files, ledger, base=None, base_files=None, base_ledger=None):
    errors = [str(e) for e in head['errors']]
    errors += ['unresolved discovery: ' + json.dumps(x, sort_keys=True) for x in head['leads'] if not x['resolved']]
    meaningful = head['physical_count']
    retained = Counter(t['fingerprint'] for t in head['tests'])
    records = {}
    for row in ledger.get('tests', []):
        fp = row.get('fingerprint')
        if not isinstance(fp, str):
            errors.append('invalid admission fingerprint type')
            continue
        if fp in records:
            errors.append('duplicate admission fingerprint: ' + fp)
            continue
        records[fp] = row
        if fp in retained and not valid_test(row, retained):
            errors.append('invalid current admission record: ' + fp)
    if base is not None:
        old = Counter(t['fingerprint'] for t in base['tests'])
        added = retained - old
        existing = retained.keys() & old.keys()
        for fingerprint, count in added.items():
            row = records.get(fingerprint, {})
            if not valid_test(row, existing) or row.get('occurrences') != count:
                errors.append('missing/invalid admission: ' + fingerprint + ' occurrences=' + str(count))
        # Prior known workload cannot be uncharged by deleting bookkeeping.
        prior_cases = {}
        for row in (base_ledger or {}).get('tests', []):
            fp, cases = row.get('fingerprint'), row.get('meaningful_cases')
            if not isinstance(fp, str) or not integer(cases):
                errors.append('invalid base admission accounting')
                continue
            prior_cases[fp] = max(prior_cases.get(fp, 1), cases)
        for fingerprint, count in retained.items():
            row = records.get(fingerprint)
            cases = row['meaningful_cases'] if row and valid_test(row, retained) else 1
            prior = prior_cases.get(fingerprint, 1) if old[fingerprint] else 1
            if prior > 1 and (not row or cases < prior):
                errors.append('prior meaningful-case accounting removed/reduced: ' + fingerprint)
            meaningful += (max(cases, prior) - 1) * count
        impacts = {}
        for row in ledger.get('impacts', []):
            if row.get('path') in impacts:
                errors.append('duplicate impact path: ' + str(row.get('path')))
            impacts[row.get('path')] = row
        old_locations = Counter((t['path'], t['fingerprint'], t['context_sha256']) for t in base['tests'])
        new_locations = Counter((t['path'], t['fingerprint'], t['context_sha256']) for t in head['tests'])
        moved_paths = {path for (path, fp, context), n in (new_locations - old_locations).items()
                       if old[fp] and retained[fp] <= old[fp]}
        for path in sorted(set(base['residues']) | set(head['residues'])):
            before, after = base['residues'].get(path), head['residues'].get(path)
            # Exact moves avoid new admission, not execution-context review.
            if path not in moved_paths and (before == after or (not before and not after)):
                continue
            row = impacts.get(path, {})
            affected = row.get('affected_tests')
            valid = reviewed(row) and all(nonempty(row.get(k)) for k in ('impact', 'execution_context'))
            valid = valid and isinstance(affected, list) and all(x in retained for x in affected)
            valid = valid and (bool(affected) or nonempty(row.get('no_tests_reason')))
            valid = valid and row.get('before_sha256') == (sha(base_files[path]) if path in base_files else None)
            valid = valid and row.get('after_sha256') == (sha(files[path]) if path in files else None)
            if not valid:
                errors.append('missing/invalid support impact: ' + path)
    else:
        # Count declared meaningful workload even when no diff was requested.
        for row in records.values():
            if row.get('fingerprint') in retained and valid_test(row, retained):
                meaningful += (row['meaningful_cases'] - 1) * retained[row['fingerprint']]
    if max(head['physical_count'], meaningful) > LIMIT:
        errors.append(f'hard budget exceeded: physical={head["physical_count"]}, meaningful={meaningful}, limit={LIMIT}')
    return errors, meaningful


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate JSON object key: ' + key)
        result[key] = value
    return result


def load_ledger(data, path):
    try:
        ledger = json.loads(data, object_pairs_hook=unique_object)
        if not isinstance(ledger, dict) or ledger.get('version') != 1 or any(not isinstance(ledger.get(k), list) or not all(isinstance(x, dict) for x in ledger[k]) for k in ('tests', 'impacts', 'classifications')):
            raise ValueError('requires version 1 and tests/impacts/classifications lists of objects')
        return ledger
    except (ValueError, UnicodeError) as exc:
        raise ValueError('invalid admissions ' + str(path) + ': ' + str(exc)) from exc


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=('inventory', 'check'))
    parser.add_argument('--repo', default='.', help='repository to inspect as data (script may be trusted elsewhere)')
    parser.add_argument('--base', help='explicit commit/ref; invalid values fail closed')
    parser.add_argument('--admissions', default=ADMISSIONS)
    args = parser.parse_args(argv)
    try:
        root = Path(git(Path(args.repo), 'rev-parse', '--show-toplevel').decode().strip()).resolve()
        ledger_path = Path(args.admissions)
        if not ledger_path.is_absolute():
            ledger_path = root / ledger_path
        safe_file(root, ledger_path)
        ledger = load_ledger(ledger_path.read_bytes(), ledger_path)
        files = snapshot(root)
        head = inventory(files, ledger)
        base_files = snapshot(root, args.base) if args.base is not None else None
        base = inventory(base_files, ledger) if base_files is not None else None
        ledger_relative = str(ledger_path.absolute().relative_to(root))
        base_ledger = None
        if base_files is not None and ledger_relative in base_files:
            base_ledger = load_ledger(base_files[ledger_relative], 'base:' + ledger_relative)
        errors, meaningful = check(head, files, ledger, base, base_files, base_ledger)
        head.pop('residues')
        output = dict(version=1, limit=LIMIT, meaningful_count=meaningful, base=args.base,
                      admissions_checked=args.base is not None, limitations=LIMITATIONS, **head)
        output['violations'] = errors
        output['ok'] = not errors
        print(json.dumps(output, indent=2, sort_keys=True))
        return int(bool(errors)) if args.command == 'check' else 0
    except (ValueError, OSError, UnicodeError, TypeError, KeyError, IndexError) as exc:
        print(json.dumps({'ok': False, 'error': str(exc)}))
        return 2


if __name__ == '__main__':
    sys.exit(main())

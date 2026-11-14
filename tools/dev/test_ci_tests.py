import contextlib
import io
import json
import os
import shlex
import subprocess
import tempfile
from pathlib import Path
import unittest
from unittest.mock import Mock

import ci_tests as ci


class InventoryTests(unittest.TestCase):
    def test_exact_identity(self):
        ci.identity('a' * 40, '1.90.0', 'a' * 40)
        ci.identity('a' * 40, 'nightly-2026-11-04', 'a' * 40)
        for sha, toolchain, head in [('main', '1.90.0', 'main'), ('a' * 40, 'stable', 'a' * 40),
                                     ('a' * 40, '1.90.0', 'b' * 40)]:
            with self.assertRaises(ValueError):
                ci.identity(sha, toolchain, head)

    def test_required_dependency_never_skips(self):
        data = ci.load_inventory()
        needs = {lane: {'result': 'success'} for lane in data['required_jobs']}
        self.assertTrue(all(r['status'] == 'PASS' for r in ci.qualification(data, needs)))
        for lane in needs:
            for result in ['failure', 'cancelled', 'skipped', None]:
                changed = {**needs, lane: {'result': result}}
                self.assertTrue(any(r['status'] == 'BLOCKED' for r in ci.qualification(data, changed)))
        self.assertTrue(all(r['status'] == 'BLOCKED' for r in ci.qualification(data, {})))

    def test_fake_execution_evidence(self):
        entry = {'id': 'fixture', 'cwd': '.', 'command': ['fixture'],
                 'positive_evidence': r'[1-9][0-9]* passed', 'reject_evidence': r'[1-9][0-9]* skipped'}
        for code, output, status in [(0, '2 passed', 'PASS'), (0, '0 passed', 'FAIL'),
                                      (0, '2 passed 1 skipped', 'FAIL'), (7, '2 passed', 'FAIL')]:
            fake = Mock(return_value=subprocess.CompletedProcess([], code, output))
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(ci.execute(entry, fake)['status'], status)
            self.assertEqual(fake.call_args.args[0], ['fixture'])
        self.assertEqual(ci.execute(entry, Mock(side_effect=FileNotFoundError('fixture')))['status'], 'BLOCKED')

    def test_named_suite_completion_rejects_compile_only_empty_and_partial(self):
        entry = next(e for e in ci.load_inventory()['entries'] if e['id'] == 'vscode-npm-test')
        complete = '\n'.join('CI_SUITE_COMPLETED ' + name + '.test.js'
                             for name in ('sidebar', 'refactDaemon', 'launchRustHealth', 'hostRelease'))
        for output, expected in [('', 'FAIL'), ('compile passed; lint ok', 'FAIL'),
                                 ('CI_SUITE_COMPLETED hostRelease.test.js', 'FAIL'),
                                 (complete, 'PASS'), (complete + '\n' + complete, 'FAIL')]:
            executor = Mock(return_value=subprocess.CompletedProcess([], 0, output))
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(ci.execute(entry, executor)['status'], expected)

    def test_vscode_execution_requires_each_declared_script_and_exit_success(self):
        package = {'scripts': {'test': ' && '.join('node ./out/' + name for name in ci.VSCODE_SUITE_FILES)}}
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / 'out').mkdir()
            for name in ci.VSCODE_SUITE_FILES:
                (directory / 'out' / name).write_text('fixture')
            for failure in range(5):
                executor = Mock(side_effect=[subprocess.CompletedProcess([], 1 if i == failure else 0) for i in range(5)])
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    self.assertEqual(ci.vscode_test(executor, package, directory), 1)
                self.assertEqual(output.getvalue().count('CI_SUITE_COMPLETED'), max(0, failure - 1))
            (directory / 'out' / ci.VSCODE_SUITE_FILES[0]).unlink()
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(ci.vscode_test(Mock(return_value=subprocess.CompletedProcess([], 0)), package, directory), 1)
            with self.assertRaises(ValueError):
                ci.vscode_test(package={'scripts': {'test': 'npm run compile'}}, directory=directory)

    def test_unavailable_and_empty_lanes(self):
        data = {'entries': [{'id': 'external', 'lane': 'external', 'required': True,
                             'unavailable': 'missing isolated capability'}]}
        fake = Mock()
        self.assertEqual(ci.run_lane(data, 'external', fake)[0]['status'], 'BLOCKED')
        data['entries'][0]['required'] = False
        self.assertEqual(ci.run_lane(data, 'external', fake)[0]['status'], 'EXCLUDED')
        fake.assert_not_called()
        with self.assertRaises(ValueError):
            ci.run_lane(data, 'missing', fake)

    def test_junit_requires_positive_fresh_non_skipped_tests(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'TEST-fixture.xml'
            for tests, skipped, expected in [(0, 0, False), (2, 1, False), (2, 0, True)]:
                path.write_text(f'<testsuite tests="{tests}" failures="0" errors="0" skipped="{skipped}">' + '<testcase/>' * tests + '</testsuite>')
                if expected:
                    self.assertEqual(ci.junit_count([path]), 2)
                else:
                    with self.assertRaises(ValueError):
                        ci.junit_count([path])
            with self.assertRaises(ValueError):
                ci.junit_count([])
            for xml in ['<testsuite tests="1"/>', '<testsuite tests="0"><testcase/></testsuite>',
                        '<testsuite><testcase><skipped/></testcase></testsuite>',
                        '<testsuite><testcase><failure/></testcase></testsuite>',
                        '<testsuite><testcase><error/></testcase></testsuite>']:
                path.write_text(xml)
                with self.assertRaises(ValueError):
                    ci.junit_count([path])
            path.write_text('<testsuite tests="1"><testsuite tests="1"><testcase/></testsuite></testsuite>')
            self.assertEqual(ci.junit_count([path]), 1)

    def test_collected_entry_identity_and_missing_evidence(self):
        data = {'required_jobs': ['gui'], 'entries': [{'id': 'fixture', 'lane': 'gui', 'kind': 'test', 'required': True}]}
        needs = {'gui': {'result': 'success'}}
        inputs = {'rust': '1.90.0', 'node': '22.23.4', 'java': '17', 'build_profile': 'ci-release', 'test_profile': 'ci-test'}
        with tempfile.TemporaryDirectory() as temporary:
            self.assertEqual(ci.collect_results(data, needs, temporary, 'a' * 40, 'b' * 64, inputs)[1]['status'], 'BLOCKED')
            report = {**ci.report_identity(), 'source_sha': 'a' * 40, 'inventory_sha256': 'b' * 64,
                      'kind': 'refact-tests-qualification', 'inputs': inputs, 'results': [{'id': 'fixture', 'status': 'PASS'}]}
            path = Path(temporary) / 'gui-report.json'
            path.write_text(json.dumps(report))
            self.assertEqual(ci.collect_results(data, needs, temporary, 'a' * 40, 'b' * 64, inputs)[1]['status'], 'PASS')
            for key in inputs:
                report['inputs'] = {**inputs, key: 'wrong'}
                path.write_text(json.dumps(report))
                self.assertEqual(ci.collect_results(data, needs, temporary, 'a' * 40, 'b' * 64, inputs)[1]['status'], 'BLOCKED')
            report['inputs'] = inputs
            report['run_id'] = 'wrong'
            path.write_text(json.dumps(report))
            self.assertEqual(ci.collect_results(data, needs, temporary, 'a' * 40, 'b' * 64, inputs)[1]['status'], 'BLOCKED')

    def test_rust_output_requires_execution(self):
        self.assertTrue(ci.test_output_valid('test result: ok. 12 passed; 0 failed'))
        self.assertFalse(ci.test_output_valid('test result: ok. 0 passed; 0 failed'))
        self.assertFalse(ci.test_output_valid('skipped: set REFACT_BROWSER_E2E=1\ntest result: ok. 12 passed'))
        self.assertFalse(ci.test_output_valid('compilation finished'))

    def test_inventory_job_entry_collision_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'inventory.json'
            path.write_text(json.dumps({'required_jobs': ['collision'], 'entries': [{'id': 'collision'}]}))
            with unittest.mock.patch.object(ci, 'INVENTORY', path), self.assertRaises(ValueError):
                ci.load_inventory()

    def test_reusable_build_blocks_export_selected_source(self):
        import re
        for name in ['agent_engine_check.yml', 'agent_browser_e2e.yml', 'engine_perf.yml']:
            text = (ci.ROOT / '.github/workflows' / name).read_text()
            self.assertIn('  REFACT_SOURCE_SHA:', text)
            for step in re.split(r'(?m)^      - ', text)[1:]:
                if '        run:' not in step:
                    continue
                block = step.split('        run:', 1)[1]
                if any(token in block for token in ['cargo test', 'cargo build', 'browser_artifact_witness.py',
                                                   'browser_windows_roadmap_qualification.ps1']):
                    expected = '$env:GITHUB_SHA = $env:REFACT_SOURCE_SHA' if 'shell: pwsh' in step else 'export GITHUB_SHA="$REFACT_SOURCE_SHA"'
                    self.assertEqual(block.splitlines()[1].strip(), expected)

    def test_inventory_contract(self):
        data = ci.load_inventory()
        for entry in data['entries']:
            for key in ['command', 'platform', 'prerequisites', 'features', 'secrets']:
                self.assertIn(key, entry)
            self.assertFalse(entry['secrets'])
            self.assertIsInstance(entry['command'], list)
            self.assertNotIn('npm pack', ' '.join(entry['command']))
            if entry['kind'] == 'test':
                self.assertIn('positive_evidence', entry)

    def test_reviewed_owned_worker_suites_are_wired_without_relaxing_other_gaps(self):
        entries = {e['id']: e for e in ci.load_inventory()['entries']}
        for name in ['test_background_notifications_e2e.py', 'test_pty_stdin_e2e.py',
                     'test_scheduler_e2e.py', 'test_hidden_roles_e2e.py']:
            entry = entries['python-' + name]
            self.assertTrue(entry['required'])
            self.assertEqual(entry['lane'], 'engine')
            self.assertEqual(entry['kind'], 'reusable')
            self.assertIn('tests/owned_engine.py', entry['command'])
            self.assertNotIn('unavailable', entry)
        self.assertNotIn('unavailable', entries['python-test_chat_session_basic.py'])
        workflow = (ci.ROOT / '.github/workflows/agent_engine_check.yml').read_text()
        self.assertIn('python3 tests/owned_engine.py --binary "$binary" --binary-sha256', workflow)
        self.assertIn('path: .refact/verification/owned-integration/', workflow)
        self.assertIn('if: always()\n        uses: actions/upload-artifact@v4', workflow)
        self.assertIn('pytest==8.4.2 pytest-asyncio==0.25.3 httpx==0.28.1', workflow)

    def test_reviewed_owned_utility_wiring_preserves_pending_gaps(self):
        entries = {e['id']: e for e in ci.load_inventory()['entries']}
        for name in ['test03_at_commands_completion.py', 'test12_tools_authorize_calls.py',
                     'test_knowledge_graph_content_param.py', 'test_knowledge_ops.py']:
            entry = entries['python-' + name]
            self.assertTrue(entry['required'])
            self.assertEqual((entry['lane'], entry['kind'], entry['cwd']),
                             ('engine', 'reusable', 'refact-agent/engine'))
            self.assertIn('tests/owned_utility_tests.py', entry['command'])
            self.assertNotIn('unavailable', entry)
        for entry in entries.values():
            if 'tests/owned_engine.py' in entry['command']:
                self.assertEqual(entry['cwd'], 'refact-agent/engine')
        for name in ['basic', 'abort', 'attachments', 'editing', 'errors', 'reliability']:
            self.assertNotIn('unavailable', entries['python-test_chat_session_' + name + '.py'])
        workflow = (ci.ROOT / '.github/workflows/agent_engine_check.yml').read_text()
        self.assertIn('python3 tests/owned_utility_tests.py --binary "$binary" --binary-sha256', workflow)
        self.assertIn('path: .refact/verification/owned-utility/', workflow)
        self.assertIn('name: Retain owned utility evidence\n        if: always()', workflow)

    def test_reviewed_owned_browser_benchmark_is_required_and_hash_pinned(self):
        import re
        data = ci.load_inventory()
        entry = next(e for e in data['entries'] if e['id'] == 'python-test_native_browser_benchmark.py')
        self.assertTrue(entry['required'])
        self.assertEqual((entry['lane'], entry['kind'], entry['cwd'], entry['platform']),
                         ('browser', 'reusable', 'refact-agent/engine', 'linux-x64'))
        self.assertNotIn('unavailable', entry)
        self.assertEqual(entry['source'], '.github/workflows/agent_browser_e2e.yml')
        self.assertEqual(entry['command'], [
            'python3', 'tests/owned_browser_benchmark.py', '--binary', 'target/debug/refact-lsp',
            '--binary-sha256', '<sha256 of freshly built subject>', '--samples', '7',
            '--actions', '24', '--transport', 'readiness_mio', '--output',
            '../../.refact/verification/owned-browser-benchmark/details.json'])
        self.assertEqual(entry['commands'], [
            ['cargo', 'build', '--locked', '--bin', 'refact-lsp'],
            ['python3', '-m', 'pytest', '-q', 'tests/test_native_browser_benchmark.py']])
        workflow = (ci.ROOT / entry['source']).read_text()
        steps = re.split(r'(?m)^      - ', workflow)[1:]
        build = next(s for s in steps if s.startswith('name: Build fresh owned browser benchmark'))
        benchmark = next(s for s in steps if s.startswith('name: Run required owned browser benchmark'))
        self.assertIn('cargo build --locked --bin refact-lsp\n', build)
        self.assertNotIn('--profile', build)
        self.assertLess(workflow.index(build), workflow.index(benchmark))
        self.assertIn('binary="$PWD/target/debug/refact-lsp"', benchmark)
        self.assertIn('binary_sha256="$(sha256sum "$binary" | cut -d \' \' -f 1)"', benchmark)
        self.assertIn('set -o pipefail', benchmark)
        self.assertIn('xvfb-run -a python3 tests/owned_browser_benchmark.py --binary "$binary" '
                      '--binary-sha256 "$binary_sha256" --samples 7 --actions 24 '
                      '--transport readiness_mio --output "$evidence/details.json" '
                      '2>&1 | tee "$evidence/runner.log"', benchmark)
        self.assertNotIn('continue-on-error', benchmark)
        self.assertNotIn('if:', benchmark)
        self.assertIn('transport_observed=unverified', benchmark)
        self.assertIn('uses: actions/setup-python@v5\n        with:\n          python-version: "3.11"', workflow)
        self.assertIn('python3 -m pip install aiohttp==3.11.18 pytest==8.4.2 '
                      'pytest-asyncio==0.25.3 httpx==0.28.1', workflow)
        self.assertIn('run: python3 -m pytest -q tests/test_native_browser_benchmark.py', workflow)
        upload = next(s for s in steps if s.startswith('name: Retain owned browser benchmark evidence'))
        self.assertIn('if: always()\n        uses: actions/upload-artifact@v4', upload)
        self.assertIn('path: ' + entry['evidence_path'], upload)
        # Successful browser job is real reusable evidence, never a parser self-check substitute.
        needs = {lane: {'result': 'success'} for lane in data['required_jobs']}
        with tempfile.TemporaryDirectory() as temporary:
            reports = ci.collect_results(data, needs, temporary, 'a' * 40, 'b' * 64, {})
        rows = {row['id']: row for row in reports}
        self.assertEqual(rows[entry['id']]['status'], 'PASS')
        gaps = {e['id'] for e in data['entries'] if e['required'] and e.get('unavailable')}
        self.assertEqual(gaps, set())

    def test_approved_chat_suites_are_required_and_job_backed(self):
        import yaml
        data = ci.load_inventory()
        entries = {entry['id']: entry for entry in data['entries']}
        chat_ids = {'python-test_chat_session_' + name + '.py'
                    for name in ['basic', 'abort', 'attachments', 'editing', 'errors', 'reliability']}
        for name in chat_ids:
            entry = entries[name]
            self.assertTrue(entry['required'])
            self.assertNotIn('unavailable', entry)
            self.assertEqual((entry['lane'], entry['kind'], entry['cwd'], entry['platform']),
                             ('engine', 'reusable', 'refact-agent/engine', 'linux-x64'))
            self.assertEqual(entry['command'], [
                'python3', 'tests/owned_chat_sessions.py', '--binary', 'target/ci-test/refact-lsp',
                '--binary-sha256', '<sha256 of freshly built subject>'])
            self.assertEqual(entry['source'], 'refact-agent/engine/tests/' + name.removeprefix('python-'))
            self.assertEqual(entry['evidence_path'], '.refact/verification/owned-chat-sessions/')
        workflow = yaml.safe_load((ci.ROOT / '.github/workflows/agent_engine_check.yml').read_text())
        steps = workflow['jobs']['check']['steps']
        build = next(step for step in steps if step.get('name') == 'Build owned tool-job fixture subject')
        chat = next(step for step in steps if step.get('name') == 'Run required owned chat session suites')
        self.assertIn('export GITHUB_SHA="$REFACT_SOURCE_SHA"', build['run'])
        self.assertIn('cargo build --locked --bin refact-lsp --profile ci-test', build['run'])
        self.assertLess(steps.index(build), steps.index(chat))
        self.assertEqual(chat['run'], 'binary="$PWD/target/ci-test/refact-lsp"\n'
                         'python3 tests/owned_chat_sessions.py --binary "$binary" '
                         '--binary-sha256 "$(sha256sum "$binary" | cut -d\' \' -f1)"\n')
        self.assertNotIn('if', chat)
        self.assertNotIn('continue-on-error', chat)
        needs = {lane: {'result': 'success'} for lane in data['required_jobs']}
        with tempfile.TemporaryDirectory() as temporary:
            rows = {r['id']: r for r in ci.collect_results(data, needs, temporary, 'a' * 40, 'b' * 64, {})}
            self.assertTrue(all(rows[name]['status'] == 'PASS' for name in chat_ids))
            # Reusable entries require successful jobs; absent lane reports still block report-backed tests.
            self.assertEqual(rows['gui-test:integration']['status'], 'BLOCKED')
            for lane in data['required_jobs']:
                missing = {key: value for key, value in needs.items() if key != lane}
                rows = {r['id']: r for r in ci.collect_results(data, missing, temporary, 'a' * 40, 'b' * 64, {})}
                self.assertEqual(rows[lane]['status'], 'BLOCKED')
                if lane == 'engine':
                    self.assertTrue(all(rows[name]['status'] == 'BLOCKED' for name in chat_ids))

    def test_no_required_gaps_and_external_audits_remain_excluded(self):
        data = ci.load_inventory()
        gaps = {e['id']: e for e in data['entries'] if e.get('unavailable') and e['required']}
        freshness = next(e for e in data['entries'] if e['id'] == 'build-freshness-fixtures')
        self.assertNotIn('unavailable', freshness)
        self.assertIn('build-freshness', data['required_jobs'])
        self.assertNotIn('gui-live-chat-fixture-gap', gaps)
        gui = next(e for e in data['entries'] if e['id'] == 'gui-test:integration')
        self.assertIn('src/__tests__/integration/chatSubscription.live.integration.test.ts', gui['command'])
        self.assertNotIn('src/__tests__/integration/chatSubscription.integration.test.ts', gui['command'])
        self.assertIn('reject_evidence', gui)
        self.assertNotIn('legacy-python-applicability-audit', gaps)
        self.assertEqual(gaps, {})
        self.assertNotIn('python-test03_at_commands_completion.py', gaps)
        self.assertTrue(all(r['status'] == 'EXCLUDED'
                            for r in ci.run_lane(data, 'external')))

    def test_lane_profile_remains_data_in_shell(self):
        workflow = (ci.ROOT / '.github/workflows/tests.yml').read_text()
        commands = [line.strip().removeprefix('run: ') for line in workflow.splitlines()
                    if 'run: python3 tools/dev/ci_tests.py ' in line]
        self.assertEqual(len(commands), 5)
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            sentinel = directory / 'injected'
            payload = '$(touch ' + shlex.quote(str(sentinel)) + ')'
            for command in commands:
                self.assertNotIn('${{', command)
                self.assertIn('--build-profile "$BUILD_PROFILE"', command)
                shell = 'python3() { printf "%s\\n" "$@"; }; ' + command
                result = subprocess.run(['bash', '-c', shell], cwd=directory,
                                        env={**os.environ, 'BUILD_PROFILE': payload},
                                        capture_output=True, text=True, check=True)
                self.assertIn(payload, result.stdout.splitlines())
                self.assertFalse(sentinel.exists())

    def test_owned_evidence_uploads_include_only_named_hidden_directories(self):
        import yaml
        expected = {
            'agent_engine_check.yml': {'owned-integration', 'owned-utility', 'owned-chat-sessions'},
            'agent_browser_e2e.yml': {'owned-browser-benchmark'},
        }
        for name, directories in expected.items():
            workflow = yaml.safe_load((ci.ROOT / '.github/workflows' / name).read_text())
            steps = [step for job in workflow['jobs'].values() for step in job.get('steps', [])
                     if step.get('uses') == 'actions/upload-artifact@v4'
                     and step.get('with', {}).get('path', '').startswith('.refact/')]
            self.assertEqual({step['with']['path'] for step in steps},
                             {f'.refact/verification/{directory}/' for directory in directories})
            for step in steps:
                self.assertEqual(step['if'], 'always()')
                self.assertIs(step['with']['include-hidden-files'], True)
                self.assertEqual(step['with']['if-no-files-found'], 'error')

    def test_workflow_dependencies_and_sha(self):
        workflow = (ci.ROOT / '.github/workflows/tests.yml').read_text()
        for lane in ci.load_inventory()['required_jobs']:
            self.assertIn(f'  {lane}:', workflow)
        self.assertIn('needs: [resolve, engine, browser, windows-browser, performance, gui, vscode, jetbrains, tooling, build-freshness]', workflow)
        self.assertEqual(workflow.count('source_sha: ${{ needs.resolve.outputs.source_sha }}'), 4)
        self.assertIn('if: always()', workflow)
        self.assertIn('run: cargo fetch --locked', workflow)
        self.assertIn('  pull_request:', workflow)
        self.assertIn('    branches: [main]', workflow)
        for name in ['agent_engine_check.yml', 'agent_browser_e2e.yml', 'engine_perf.yml']:
            text = (ci.ROOT / '.github/workflows' / name).read_text()
            self.assertIn('  workflow_call:', text)
            self.assertIn('persist-credentials: false', text)
            if name == 'agent_engine_check.yml':
                self.assertIn('${{ inputs.browser_artifacts_only }}-${{ inputs.browser_roadmap_windows }}', text)
            self.assertIn('test "$(git rev-parse HEAD)" = "$SOURCE_SHA"', text)

    def test_freshness_removes_local_cargo_defaults_before_cache_and_cargo(self):
        import yaml
        workflow = yaml.safe_load((ci.ROOT / '.github/workflows/tests.yml').read_text())
        steps = workflow['jobs']['build-freshness']['steps']
        checkout = next(i for i, step in enumerate(steps) if step.get('uses') == 'actions/checkout@v4')
        cleanup = next(i for i, step in enumerate(steps) if step.get('name') == 'Disable local nightly Cargo defaults for stable fixtures')
        cache = next(i for i, step in enumerate(steps) if step.get('uses') == 'swatinem/rust-cache@v2')
        self.assertLess(checkout, cleanup)
        self.assertLess(cleanup, cache)
        self.assertNotIn('working-directory', steps[cleanup])
        self.assertEqual(steps[cleanup]['run'], 'rm -f .cargo/config.toml')
        self.assertTrue(all(cleanup < i for i, step in enumerate(steps) if 'cargo ' in step.get('run', '')))
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / '.cargo').mkdir()
            local = root / '.cargo/config.toml'
            local.write_text('[unstable]\nprofile-rustflags = true\n')
            sentinel = root / 'Cargo.lock'
            sentinel.write_text('unchanged')
            subprocess.run(['bash', '-eu', '-c', steps[cleanup]['run']], cwd=root, check=True)
            self.assertFalse(local.exists())
            self.assertEqual(sentinel.read_text(), 'unchanged')

    def test_gui_runner_matches_locked_playwright_supported_platform(self):
        import yaml
        workflow = yaml.safe_load((ci.ROOT / '.github/workflows/tests.yml').read_text())
        gui = workflow['jobs']['gui']
        self.assertEqual(gui['runs-on'], 'ubuntu-24.04')
        steps = gui['steps']
        install = next(i for i, step in enumerate(steps) if step.get('run') == 'npx --no-install playwright install --with-deps chromium')
        execute = next(i for i, step in enumerate(steps) if step.get('name') == 'Execute required gui inventory')
        self.assertLess(install, execute)
        self.assertEqual(steps[install]['working-directory'], 'refact-agent/gui')
        self.assertNotIn('continue-on-error', steps[install])

    def test_performance_uses_the_engine_test_stack_without_skipping_fixtures(self):
        import yaml
        workflow = yaml.safe_load((ci.ROOT / '.github/workflows/engine_perf.yml').read_text())
        self.assertEqual(str(workflow['env']['RUST_MIN_STACK']), '16777216')
        steps = workflow['jobs']['engine-perf']['steps']
        performance = next(step for step in steps if step.get('name') == 'Run engine performance benchmarks')
        coordination = next(step for step in steps if step.get('name') == 'Run all ignored coordination benchmarks')
        self.assertIn('chat::perf_harness --include-ignored --test-threads=1 --nocapture', performance['run'])
        self.assertIn('coordination:: --ignored --test-threads=1 --nocapture', coordination['run'])
        for step in [performance, coordination]:
            self.assertNotIn('continue-on-error', step)
            self.assertNotIn('if', step)


if __name__ == '__main__':
    unittest.main()

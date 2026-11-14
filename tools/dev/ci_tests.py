import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import unittest
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parents[2]
INVENTORY = Path(__file__).with_name('ci_test_inventory.json')
SHA = re.compile(r'[0-9a-f]{40}')


def identity(source_sha, toolchain, head):
    if not SHA.fullmatch(source_sha) or head != source_sha:
        raise ValueError('source_sha must be a full lowercase SHA matching checkout HEAD')
    if not re.fullmatch(r'(?:\d+\.\d+\.\d+|nightly-\d{4}-\d{2}-\d{2})', toolchain):
        raise ValueError('toolchain must be an exact Rust release, not a moving channel')


def load_inventory():
    data = json.loads(INVENTORY.read_text())
    ids = data['required_jobs'] + [entry['id'] for entry in data['entries']]
    if len(ids) != len(set(ids)):
        raise ValueError('duplicate inventory IDs')
    return data


def execute(entry, executor=subprocess.run):
    try:
        result = executor(entry['command'], cwd=ROOT / entry['cwd'],
                          env={**os.environ, **entry.get('env', {})},
                          text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                          timeout=entry.get('timeout', 3600), check=False)
        output = result.stdout or ''
        print(output, flush=True)
        passed = result.returncode == 0
        if entry.get('positive_evidence'):
            passed = passed and bool(re.search(entry['positive_evidence'], output))
        for pattern in entry.get('required_evidence', []):
            passed = passed and len(re.findall(pattern, output)) == 1
        if entry.get('reject_evidence'):
            passed = passed and not re.search(entry['reject_evidence'], output)
        return {'id': entry['id'], 'status': 'PASS' if passed else 'FAIL',
                'returncode': result.returncode, 'output_sha256': hashlib.sha256(output.encode()).hexdigest()}
    except (OSError, subprocess.TimeoutExpired) as error:
        return {'id': entry['id'], 'status': 'BLOCKED', 'reason': str(error)}


def run_lane(data, lane, executor=subprocess.run):
    entries = [entry for entry in data['entries'] if entry['lane'] == lane]
    if not entries:
        raise ValueError('empty or unknown lane')
    results = []
    for entry in entries:
        if entry.get('unavailable'):
            results.append({'id': entry['id'], 'status': 'BLOCKED' if entry['required'] else 'EXCLUDED',
                            'reason': entry['unavailable']})
        else:
            results.append(execute(entry, executor))
    return results


def qualification(data, needs):
    return [{'id': lane, 'status': 'PASS' if needs.get(lane, {}).get('result') == 'success' else 'BLOCKED'}
            for lane in data['required_jobs']]


def report_identity():
    return {key: os.environ.get(env, '') for key, env in
            [('repository_id', 'GITHUB_REPOSITORY_ID'), ('run_id', 'GITHUB_RUN_ID'), ('run_attempt', 'GITHUB_RUN_ATTEMPT')]}


def collect_results(data, needs, directory, source_sha, inventory_hash, inputs):
    results = qualification(data, needs)
    reports = {}
    for lane in ('gui', 'vscode', 'jetbrains', 'tooling', 'build-freshness'):
        paths = list(Path(directory).rglob(lane + '-report.json'))
        if len(paths) != 1:
            continue
        report = json.loads(paths[0].read_text())
        expected = {'source_sha': source_sha, 'inventory_sha256': inventory_hash,
                    'kind': 'refact-tests-qualification', 'inputs': inputs, **report_identity()}
        if any(report.get(key) != value for key, value in expected.items()):
            continue
        entries = report.get('results', [])
        expected_ids = {e['id'] for e in data['entries'] if e['lane'] == lane}
        if len(entries) != len(expected_ids) or {e.get('id') for e in entries} != expected_ids:
            continue
        reports[lane] = {r['id']: r for r in entries}
    for entry in data['entries']:
        if entry.get('unavailable'):
            result = {'id': entry['id'], 'status': 'BLOCKED' if entry['required'] else 'EXCLUDED',
                      'reason': entry['unavailable']}
        elif entry['lane'] in reports:
            result = reports[entry['lane']][entry['id']]
            if entry['required'] and result.get('status') != 'PASS':
                result = {'id': entry['id'], 'status': 'BLOCKED', 'reason': 'mandatory entry did not pass'}
        elif entry['kind'] == 'reusable':
            result = {'id': entry['id'], 'status': 'PASS' if needs.get(entry['lane'], {}).get('result') == 'success' else 'BLOCKED',
                      'evidence_source': 'job', 'job': entry['lane']}
        else:
            result = {'id': entry['id'], 'status': 'BLOCKED', 'reason': 'missing or invalid current lane report'}
        results.append(result)
    return results


def test_output_valid(output):
    return bool(re.search(r'test result: ok\. [1-9][0-9]* passed', output)) and 'skipped: set ' not in output


VSCODE_SUITE_FILES = ('sidebar.test.js', 'refactDaemon.test.js', 'launchRustHealth.test.js', 'hostRelease.test.js')


def vscode_test(executor=subprocess.run, package=None, directory=None):
    directory = directory or ROOT / 'plugins/vscode'
    package = package or json.loads((directory / 'package.json').read_text())
    expected = ' && '.join('node ./out/' + name for name in VSCODE_SUITE_FILES)
    if package['scripts']['test'] != expected:
        raise ValueError('VSCode declared test command drifted; review named suite inventory')
    commands = [['npm', 'run', 'pretest']] + [['node', './out/' + name] for name in VSCODE_SUITE_FILES]
    for index, command in enumerate(commands):
        if index and not (directory / 'out' / VSCODE_SUITE_FILES[index - 1]).is_file():
            return 1
        result = executor(command, cwd=directory, env=dict(os.environ), check=False)
        if result.returncode:
            return 1
        if index:
            print('CI_SUITE_COMPLETED ' + VSCODE_SUITE_FILES[index - 1], flush=True)
    return 0


def junit_count(paths):
    count = 0
    for path in paths:
        root = ET.parse(path).getroot()
        for suite in root.iter('testsuite'):
            cases = suite.findall('testcase')
            actual = len(suite.findall('.//testcase'))
            if int(suite.attrib.get('tests', str(actual))) != actual:
                raise ValueError('JUnit declared count differs from testcase evidence')
            if any(int(suite.attrib.get(key, '0')) for key in ('failures', 'errors', 'skipped', 'disabled')):
                raise ValueError('JUnit suite failed or skipped tests')
            for case in cases:
                if any(case.find(tag) is not None for tag in ('failure', 'error', 'skipped')):
                    raise ValueError('JUnit testcase failed or skipped')
            count += len(cases)
    if count <= 0:
        raise ValueError('JUnit reports contain zero tests')
    return count


def jetbrains_test():
    directory = ROOT / 'plugins/intellij'
    reports = directory / 'build/test-results/test'
    if reports.is_symlink() or any(p.is_symlink() for p in (directory / 'build', reports.parent)):
        raise ValueError('Refuse symlinked Gradle reports')
    if reports.exists():
        shutil.rmtree(reports)
    result = subprocess.run(['./gradlew', 'test', 'check', 'verifyPlugin', '--rerun-tasks', '--no-daemon', '--console=plain'],
                            cwd=directory, env=dict(os.environ), check=False)
    if result.returncode:
        return 1
    count = junit_count(sorted(reports.glob('TEST-*.xml')))
    print(f'CI_JUNIT_COMPLETED {count}', flush=True)
    return 0


def strict_unittest(paths):
    suite = unittest.TestSuite()
    for path in paths:
        suite.addTests(unittest.TestLoader().discover(str(Path(path).parent), pattern=Path(path).name))
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    return 0 if result.wasSuccessful() and result.testsRun > 0 and not result.skipped else 1


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument('--source-sha', required=True)
    parser.add_argument('--toolchain', required=True)
    parser.add_argument('--node-version', default='22')
    parser.add_argument('--build-profile', default='ci-release')
    parser.add_argument('--lane')
    parser.add_argument('--needs-json')
    parser.add_argument('--reports-dir', default='lane-reports')
    parser.add_argument('--report', required=True)
    args = parser.parse_args(argv)
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    identity(args.source_sha, args.toolchain, head)
    data = load_inventory()
    inventory_hash = hashlib.sha256(INVENTORY.read_bytes()).hexdigest()
    lane_inputs = {'rust': args.toolchain, 'node': args.node_version, 'java': '17',
                   'build_profile': args.build_profile, 'test_profile': 'ci-test'}
    results = collect_results(data, json.loads(args.needs_json), args.reports_dir, args.source_sha, inventory_hash, lane_inputs) if args.needs_json else run_lane(data, args.lane)
    build_metadata = {}
    if args.needs_json:
        from runtime_manifest import compute_build_inputs, build_inputs_digest
        inputs = compute_build_inputs(ROOT, args.toolchain, args.node_version, args.build_profile)
        build_metadata = {'build_inputs': inputs, 'build_inputs_digest': build_inputs_digest(inputs)}
    report = {**build_metadata, **report_identity(), 'schema': 1, 'schema_version': 1, 'kind': 'refact-tests-qualification', 'source_sha': args.source_sha,
              'inputs': lane_inputs,
              'inventory_sha256': inventory_hash,
              'results': results, 'scope': 'selected supported lanes; exclusions are not passes',
              'status': 'PASS' if all(r['status'] in ('PASS', 'EXCLUDED') for r in results) else 'BLOCKED'}
    report['excluded_coverage'] = [r for r in results if r['status'] == 'EXCLUDED']
    report['coverage_complete'] = report['status'] == 'PASS'
    report['tests'] = 'passed' if report['status'] == 'PASS' else 'blocked'
    report['test_profile'] = 'ci-test'
    Path(args.report).write_text(json.dumps(report, indent=2) + '\n')
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    if len(sys.argv) > 1 and sys.argv[1] == 'jetbrains-test':
        sys.exit(jetbrains_test())
    if len(sys.argv) > 1 and sys.argv[1] == 'vscode-test':
        sys.exit(vscode_test())
    if len(sys.argv) > 1 and sys.argv[1] == 'verify-output':
        output = Path(sys.argv[2]).read_text()
        sys.exit(0 if test_output_valid(output) else 1)
    if len(sys.argv) > 1 and sys.argv[1] == 'unittest':
        sys.exit(strict_unittest(sys.argv[2:]))
    sys.exit(main())

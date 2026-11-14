import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import traceback
import sys
import threading
import time
from types import SimpleNamespace

from owned_engine import Engine, verified_copy, write_fixture
from test_native_browser_benchmark import run, sha, finish_cleanup


def proc_identity(pid):
    try:
        path = Path('/proc') / str(pid)
        fields = (path / 'stat').read_text().rsplit(')', 1)[1].split()
        return dict(pid=pid, ppid=int(fields[1]), starttime=fields[19], state=fields[0],
                    command=(path / 'cmdline').read_bytes().replace(b'\0', b' ').decode(errors='replace'))
    except (FileNotFoundError, ProcessLookupError, PermissionError):
        return None


class ChromeDescendants:
    def __init__(self, pid):
        self.pid = pid
        self.identities = {}
        self.event = threading.Event()
        self.thread = threading.Thread(target=self.watch)
        self.error = None

    def scan(self):
        processes = [identity for path in Path('/proc').iterdir() if path.name.isdigit()
                     if (identity := proc_identity(int(path.name))) is not None]
        descendants = {self.pid}
        while True:
            found = {item['pid'] for item in processes if item['ppid'] in descendants}
            if found <= descendants:
                break
            descendants |= found
        for item in processes:
            if item['pid'] != self.pid and item['pid'] in descendants and 'chrome' in item['command'].lower():
                self.identities[(item['pid'], item['starttime'])] = item

    def watch(self):
        try:
            while not self.event.is_set():
                self.scan()
                self.event.wait(.05)
        except BaseException as error:
            self.error = error

    def finish(self, result):
        self.event.set()
        self.thread.join(timeout=10)
        assert not self.thread.is_alive(), 'descendant observer did not stop'
        if self.error:
            raise self.error
        deadline = time.monotonic() + 10
        remaining = []
        while True:
            remaining = [current for (pid, start) in self.identities
                         if (current := proc_identity(pid)) and current['starttime'] == start]
            if not remaining or time.monotonic() >= deadline:
                break
            time.sleep(.1)
        result['chrome_descendant_identities'] = list(self.identities.values())
        result['chrome_descendants_remaining'] = remaining
        result['chrome_cleanup_confirmed'] = bool(self.identities) and not remaining
        assert not remaining, f'Known owned Chrome descendants remain: {remaining}'
        assert self.identities, 'No owned Chrome descendant identity observed'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--binary-sha256', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--samples', type=int, default=7)
    parser.add_argument('--actions', type=int, default=24)
    parser.add_argument('--transport', choices=['polling_10ms', 'readiness_mio'], required=True)
    args = parser.parse_args()
    args.binary = args.binary.resolve(strict=True)
    args.output = args.output.resolve()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    summary_path = args.output.with_suffix('.summary.json')
    result = dict(schema_version=1, lane='owned-native-browser', status='failed',
                  binary_sha256=args.binary_sha256, skipped=0, metrics_policy='report-only',
                  compiled_source_provenance='explicit pinned binary; see parent build receipt',
                  samples=args.samples, actions=args.actions, warmup_samples=1,
                  transport_requested=args.transport, transport_observed='unverified',
                  source_head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip())
    result['lane_source_sha256'] = {name: sha(Path(__file__).with_name(name)) for name in (
        'owned_engine.py', 'owned_browser_benchmark.py', 'test_native_browser_benchmark.py')}
    engine = None
    try:
        with tempfile.TemporaryDirectory(prefix='owned-browser-') as directory:
            root = Path(directory)
            binary = verified_copy(args.binary, args.binary_sha256, root / 'refact-lsp')
            result['version'] = subprocess.check_output([str(binary), '--version'], text=True).strip()
            workspace, config, cache = (root / name for name in ('workspace', 'config', 'cache'))
            write_fixture(workspace, config, cache, 9)
            (config / '.config/refact/browser.yaml').write_text(
                'launch:\n  chrome_path: /usr/bin/google-chrome\n  headless: true\n'
                '  chromium_sandbox: true\n  gpu: disabled\n'
                '  extra_args: ["--disable-background-networking", "--no-first-run"]\n'
                'viewport:\n  desktop: {width: 1280, height: 720, scale_factor: 1.0}\n')
            launcher = root / 'profiled-engine'
            launcher.write_text('#!/bin/sh\nexport REFACT_BROWSER_PROFILE=1\nexec "'
                                + str(binary) + '" "$@"\n')
            launcher.chmod(0o700)
            engine = Engine(launcher, workspace, cache, config)
            observer = None
            try:
                engine.start()
                observer = ChromeDescendants(engine.process.pid)
                observer.thread.start()
                result.update(pid=engine.process.pid, port=engine.port, workspace=str(workspace))
                bench_args = SimpleNamespace(**{**vars(args), 'binary': binary})
                measured = run(engine, bench_args)
                assert measured['cleanup_owned_tabs_remaining'] == 0
                assert measured['fixture_closed'] is True
                assert all(len(row['samples']) == args.samples for row in measured['results'].values())
                result.update(status='passed', results=measured['results'],
                              exact_effects=measured['exact_effects'], profile_present=measured['profile_present'],
                              observed_profile_count=sum(len(row['profiles']) for row in measured['profiles']))
            finally:
                primary_error = sys.exception()
                errors = []
                for callback in (
                    engine.stop,
                    lambda: result.update(worker_cleanup_confirmed=engine.process is None or engine.process.poll() is not None),
                    lambda: observer.finish(result) if observer else None,
                    lambda: shutil.copyfile(engine.log_path, args.output.with_suffix('.engine.log')) if getattr(engine, 'log_path', None) else None,
                ):
                    try:
                        callback()
                    except BaseException as error:
                        errors.append(error)
                result['cleanup_errors'] = [repr(error) for error in errors]
                finish_cleanup(primary_error, errors)
    except BaseException as error:
        result.update(status='failed', error=repr(error), traceback=traceback.format_exc())
    finally:
        summary_path.write_text(json.dumps(result, indent=2, allow_nan=False) + '\n')
        print(json.dumps(result, allow_nan=False), flush=True)
    raise SystemExit(0 if result['status'] == 'passed' else 1)


if __name__ == '__main__':
    main()

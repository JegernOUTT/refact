"""Required owned native-browser benchmark; entry point: owned_browser_benchmark.py."""
import hashlib
import json
import math
from pathlib import Path
import statistics
import sys
import subprocess
import threading
import time
import urllib.request
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HTML = b'''<!doctype html><title>Local pinned benchmark</title>
<button id="click" onclick="window.clicks++">Count</button>
<form onsubmit="event.preventDefault();window.submits++;window.saved=document.querySelector('#text').value">
<input id="text"><button id="save">Save</button></form>
<script>window.clicks=0;window.submits=0;window.saved='';</script>'''


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class Client:
    def __init__(self, engine):
        from owned_engine import owns_listener
        assert engine.process.poll() is None and owns_listener(engine.process.pid, engine.port)
        self.engine = f'http://127.0.0.1:{engine.port}'
        self.chat = 'isolated-perf-' + uuid.uuid4().hex
        self.generation = None
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.profiles = []

    def call(self, steps, **options):
        assert 0 < len(steps) <= 64
        call = 'call_' + uuid.uuid4().hex
        request = dict(page_context='none', network='none', attach_screenshot=False,
                       steps=steps, **options)
        payload = dict(messages=[dict(role='assistant', content='', tool_calls=[dict(
            id=call, type='function', function=dict(name='chrome', arguments=json.dumps(dict(request=request))))])],
            n_ctx=262144, maxgen=4096, subchat_tool_parameters={}, postprocess_parameters={},
            model_name='', chat_id=self.chat)
        req = urllib.request.Request(self.engine + '/v1/tools-execute',
                                     data=json.dumps(payload).encode(), headers={'Content-Type': 'application/json'})
        start = time.perf_counter_ns()
        with self.http.open(req, timeout=120) as response:
            envelope = json.loads(response.read(24 * 1024 * 1024))
        resolved = []
        for message in envelope['messages']:
            job = message.get('tool_job')
            if not job:
                resolved.append(message)
                continue
            assert message.get('tool_call_id', job.get('tool_call_id')) == call, message
            deadline = time.monotonic() + 120
            while time.monotonic() < deadline:
                with self.http.open(self.engine + f"/v1/chats/{self.chat}/tool-jobs/{job['job_id']}", timeout=20) as response:
                    state = json.loads(response.read(24 * 1024 * 1024))
                if state.get('result') is not None:
                    assert state['status'] == 'succeeded', state
                    resolved.extend(state['result']['messages'])
                    break
                time.sleep(.01)
            else:
                raise AssertionError(f'native browser job timed out: {job}')
        elapsed = (time.perf_counter_ns() - start) / 1e9
        messages = [m for m in resolved if m.get('tool_call_id') == call and m['role'] == 'tool']
        assert all(not m.get('tool_failed') for m in messages), messages
        assert len(messages) == 1
        content = messages[0]['content']
        if isinstance(content, list):
            texts = [e.get('m_content', e.get('text')) for e in content if e.get('m_type', e.get('type')) == 'text']
            assert len(texts) == 1
            content = texts[0]
        report = json.loads(content)
        assert report.get('ok') is True, report
        completion = report['completion']
        assert not any(completion.get(k) for k in ['failed', 'unknown', 'skipped', 'incomplete']), completion
        assert completion['steps_requested'] == completion['steps_reported'] == len(steps)
        assert len(report['steps']) == len(steps) and all(s['ok'] for s in report['steps'])
        generation = report['session_generation']
        if self.generation is None:
            self.generation = generation
        assert generation == self.generation, 'generation changed during baseline'
        profile = report.get('browser_profile', envelope.get('browser_profile'))
        assert isinstance(profile, dict) and profile, 'browser_profile evidence required for every completed call'
        self.profiles.append(profile)
        return report, elapsed


    def stop_browser(self):
        request = urllib.request.Request(self.engine + '/v1/browser/stop',
            data=json.dumps({'chat_id': self.chat}).encode(),
            headers={'Content-Type': 'application/json'})
        with self.http.open(request, timeout=30) as response:
            assert response.status == 200


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def close_owned_tabs(client, owned, attempt):
    for tab in sorted(owned):
        def close(tab=tab):
            client.call([action('close_tab', tab=dict(type='id', id=tab))])
            owned.remove(tab)
        attempt(close)


def finish_cleanup(primary_error, errors):
    if primary_error is not None:
        for error in errors:
            primary_error.add_note(f'Cleanup failed: {error!r}')
    elif errors:
        raise BaseExceptionGroup('Benchmark cleanup failed', errors)


def action(name, **kw):
    return dict(action=name, **kw)


def locator(selector):
    return dict(by='css', value=selector)


def oracle(expression):
    return action('eval', expression="(()=>{if(!(" + expression + "))throw Error('benchmark exact effect');return true})()")


def summary(values):
    ordered = sorted(values)
    return dict(samples=values, median=statistics.median(values), p95=ordered[math.ceil(.95 * len(values)) - 1])


def run(engine, args):
    assert args.samples >= 3 and 2 <= args.actions <= 30 and args.actions % 2 == 0

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            self.send_header('Content-Length', str(len(HTML)))
            self.end_headers()
            self.wfile.write(HTML)

        def log_message(self, *_args):
            pass

    server = thread = c = None
    owned = set()
    output = {}
    try:
        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever)
        c = Client(engine)
        rows = {name: [] for name in ['click', 'form', 'tab_churn', 'empty_report']}
        profiles = []
        output = dict(timer_scope='client perf_counter_ns HTTP send through native-job completion/response parse including 10ms receipt polling; setup/oracles excluded except final exact-effect oracle per batch',
                      warmup_samples=1, browser_version=subprocess.check_output(['/usr/bin/google-chrome', '--version'], text=True).strip(),
                      fixture_sha256=hashlib.sha256(HTML).hexdigest(), binary_sha256=sha(args.binary),
                      build_profile='explicit prebuilt binary; compiled-source provenance unverified', source_head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                      source_hashes_scope='observed at run time, not compiled-source proof; see external build/prebuild pins',
                       source_hashes={}, requested_actions_per_sample=args.actions,
                      config=dict(headless=True, chromium_sandbox=True, gpu='disabled', viewport='1280x720', transport_requested=args.transport, transport_observed='unverified'))
        source_paths = ['crates/refact-browser/src', 'vendor/headless_chrome/src/browser/transport', 'src/integrations/browser_controller.rs', 'src/tools/tool_chrome.rs']
        for source in source_paths:
            path = Path(__file__).resolve().parents[1] / source
            assert path.exists(), path
            for file in sorted(path.rglob('*') if path.is_dir() else [path]):
                if file.is_file():
                    output['source_hashes'][str(file)] = sha(file)
        url = f'http://127.0.0.1:{server.server_port}/'
        thread.start()
        report, _ = c.call([action('open_tab', url=url)])
        primary = report['steps'][0]['data']['tab_id']
        owned.add(primary)
        initial_report, _ = c.call([action('list_tabs')])
        initial_tabs = {t.get('id', t.get('tab_id')) for t in initial_report['steps'][0]['data']['tabs']}
        assert primary in initial_tabs
        preexisting = initial_tabs - owned
        output['initial_unowned_tab_count'] = len(preexisting)
        for sample in range(args.samples + 1):
            c.call([action('eval', expression="window.clicks=0;window.submits=0;window.saved='';true")])
            batches = {
                'click': [action('click', locator=locator('#click')) for _ in range(args.actions)] + [oracle(f'window.clicks==={args.actions}')],
                'form': [s for i in range(args.actions // 2) for s in [action('fill', locator=locator('#text'), text=f'value-{i}'), action('click', locator=locator('#save'))]] + [oracle(f'window.submits==={args.actions // 2} && window.saved==="value-{args.actions // 2 - 1}"')],
                'empty_report': [oracle('true')],
            }
            for name, steps in batches.items():
                before = len(c.profiles)
                _, elapsed = c.call(steps)
                if sample:
                    rows[name].append(elapsed)
                    profiles.append(dict(sample=sample, workload=name, profiles=c.profiles[before:]))
            before = len(c.profiles)
            start = time.perf_counter_ns()
            for _ in range(2):
                report, _ = c.call([action('open_tab', url=url)])
                tab = report['steps'][0]['data']['tab_id']
                assert tab not in owned
                owned.add(tab)
                c.call([action('close_tab', tab=dict(type='id', id=tab))])
                owned.remove(tab)
            report, _ = c.call([action('list_tabs')])
            tabs = report['steps'][0]['data']['tabs']
            assert {t.get('id', t.get('tab_id')) for t in tabs} == owned | preexisting, tabs
            if sample:
                rows['tab_churn'].append((time.perf_counter_ns() - start) / 1e9)
                profiles.append(dict(sample=sample, workload='tab_churn', profiles=c.profiles[before:]))
        output.update(results={k: summary(v) for k, v in rows.items()}, profiles=profiles,
                      exact_effects=dict(clicks=args.actions * (args.samples + 1), submits=(args.actions // 2) * (args.samples + 1), tabs_opened_and_closed=2 * (args.samples + 1)),
                      profile_present=bool(c.profiles))
    finally:
        primary_error = sys.exception()
        errors = []
        def attempt(callback):
            try:
                callback()
            except BaseException as error:
                errors.append(error)
        if c is not None:
            close_owned_tabs(c, owned, attempt)
            output['cleanup_owned_tabs_remaining'] = len(owned)
            if c.generation is not None:
                attempt(c.stop_browser)
        if server is not None:
            if thread is not None and thread.ident is not None:
                attempt(server.shutdown)
                attempt(lambda: thread.join(timeout=10))
                attempt(lambda: require(not thread.is_alive(), 'fixture thread still alive'))
            attempt(server.server_close)
            output['fixture_closed'] = server.socket.fileno() == -1
        output['profile_present'] = bool(c and c.profiles)
        output['cleanup_errors'] = [repr(error) for error in errors]
        attempt(lambda: Path(args.output).write_text(json.dumps(output, indent=2, allow_nan=False)))
        finish_cleanup(primary_error, errors)
    print(json.dumps({k: v for k, v in output.items() if k in ('results', 'profile_present', 'exact_effects')}, indent=2))
    assert output.get('profile_present'), 'instrumented baseline requires browser_profile evidence'
    return output


def test_cleanup_attempts_every_tab_and_retains_primary():
    from types import SimpleNamespace
    seen, errors = [], []
    def call(steps):
        tab = steps[0]['tab']['id']
        seen.append(tab)
        if tab in ('a', 'b'):
            raise RuntimeError(tab)
    def attempt(callback):
        try:
            callback()
        except BaseException as error:
            errors.append(error)
    owned = {'a', 'b', 'c'}
    close_owned_tabs(SimpleNamespace(call=call), owned, attempt)
    assert seen == ['a', 'b', 'c'] and owned == {'a', 'b'}
    primary = ValueError('primary')
    finish_cleanup(primary, errors)
    assert len(primary.__notes__) == 2 and str(primary) == 'primary'
    try:
        finish_cleanup(None, errors)
    except BaseExceptionGroup as error:
        assert len(error.exceptions) == 2
    else:
        raise AssertionError('cleanup errors must be fatal')


def test_early_setup_failure_closes_fixture(monkeypatch, tmp_path):
    from types import SimpleNamespace
    closed = []
    class Server:
        server_port = 1
        socket = SimpleNamespace(fileno=lambda: -1)
        def serve_forever(self):
            raise AssertionError('must not start')
        def server_close(self):
            closed.append(True)
    monkeypatch.setattr(__import__(__name__), 'ThreadingHTTPServer', lambda *args: Server())
    def fail(_engine):
        raise ValueError('early setup')
    monkeypatch.setattr(__import__(__name__), 'Client', fail)
    output = tmp_path / 'failure.json'
    try:
        run(None, SimpleNamespace(samples=7, actions=24, output=output))
    except ValueError as error:
        assert str(error) == 'early setup'
    else:
        raise AssertionError('primary failure lost')
    assert closed == [True] and json.loads(output.read_text())['fixture_closed']


if __name__ == '__main__':
    from owned_browser_benchmark import main
    main()

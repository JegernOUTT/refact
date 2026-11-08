"""Deliberate isolated-worker performance lane (not collected as a default timing test).

Run: python3 tests/test_native_browser_benchmark.py --engine http://127.0.0.1:18990
 --binary target/ci-test/refact-lsp --output /tmp/refact-browser-perf-current.json
Worker must be yours, REFACT_BROWSER_PROFILE=1, unique HOME/cache/profile/project.
No native chrome tool, MCP, external web sites, or existing daemon is used.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import statistics
import subprocess
import threading
import time
import urllib.parse
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
        parsed = urllib.parse.urlsplit(engine)
        assert parsed.scheme == 'http' and parsed.hostname == '127.0.0.1' and parsed.port in (18990, 18991) and not parsed.path.strip('/') and not parsed.query and not parsed.fragment and not parsed.username, 'refuse any unapproved/shared endpoint'
        self.engine = engine
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
        elapsed = (time.perf_counter_ns() - start) / 1e9
        messages = [m for m in envelope['messages'] if m.get('tool_call_id') == call and m['role'] == 'tool']
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
        if profile is not None:
            self.profiles.append(profile)
        return report, elapsed


def action(name, **kw):
    return dict(action=name, **kw)


def locator(selector):
    return dict(by='css', value=selector)


def oracle(expression):
    return action('eval', expression="(()=>{if(!(" + expression + "))throw Error('benchmark exact effect');return true})()")


def summary(values):
    ordered = sorted(values)
    return dict(samples=values, median=statistics.median(values), p95=ordered[math.ceil(.95 * len(values)) - 1])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', default='http://127.0.0.1:18990')
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--samples', type=int, default=7)
    parser.add_argument('--actions', type=int, default=24)
    parser.add_argument('--transport', choices=['polling_10ms', 'readiness_mio'], required=True)
    args = parser.parse_args()
    assert args.samples >= 3 and 1 <= args.actions <= 30

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            self.send_header('Content-Length', str(len(HTML)))
            self.end_headers()
            self.wfile.write(HTML)

        def log_message(self, *_args):
            pass

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever)
    thread.start()
    c = Client(args.engine)
    rows = {name: [] for name in ['click', 'form', 'tab_churn', 'empty_report']}
    profiles = []
    output = dict(timer_scope='client perf_counter_ns HTTP request send through response parse; setup/oracles excluded except final exact-effect oracle per batch',
                  warmup_samples=1, browser_version=subprocess.check_output(['/usr/bin/google-chrome', '--version'], text=True).strip(),
                  fixture_sha256=hashlib.sha256(HTML).hexdigest(), binary_sha256=sha(args.binary),
                  build_profile='ci-test API-only', source_head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                  source_hashes_scope='observed at run time, not compiled-source proof; see external build/prebuild pins',
                   source_hashes={}, requested_actions_per_sample=args.actions,
                  config=dict(headless=True, chromium_sandbox=True, gpu='disabled', viewport='1280x720', transport=args.transport))
    source_paths = ['crates/refact-browser/src', 'vendor/headless_chrome/src/browser/transport', 'src/integrations/browser_controller.rs', 'src/tools/tool_chrome.rs']
    for source in source_paths:
        path = Path(source)
        for file in sorted(path.rglob('*') if path.is_dir() else [path]):
            if file.is_file():
                output['source_hashes'][str(file)] = sha(file)
    owned = set()
    url = f'http://127.0.0.1:{server.server_port}/'
    try:
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
        try:
            for tab in list(owned):
                c.call([action('close_tab', tab=dict(type='id', id=tab))])
                owned.remove(tab)
            output['cleanup_owned_tabs_remaining'] = len(owned)
        finally:
            server.shutdown()
            thread.join()
            server.server_close()
            output['fixture_closed'] = True
            Path(args.output).write_text(json.dumps(output, indent=2))
    print(json.dumps({k: v for k, v in output.items() if k in ('results', 'profile_present', 'exact_effects')}, indent=2))
    assert output.get('profile_present'), 'instrumented baseline requires browser_profile evidence'


if __name__ == '__main__':
    main()

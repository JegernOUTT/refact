"""Explicit, isolated actual-native roadmap qualification; never a default unit test.

Run: python test_native_browser_roadmap.py --url http://127.0.0.1:PORT --output-dir DIR
The URL must identify an operator-owned candidate, never the shared :8001 service.
"""
import argparse
import base64
import tempfile
import unittest
import json
import threading
import time
import traceback
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from test_native_browser_smoke import require, text_report, check_completion


HTML = '''<!doctype html><title>Roadmap fixture</title>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<section aria-label="Scope Alpha"><button id="inside" onclick="++window.scoped">Scoped Save</button></section>
<button id="outside">Scoped Save outside</button><button id="frame-target" onclick="++window.wrong">Frame Action</button>
<iframe id="child" src="/frame"></iframe><button id="tap" onclick="++window.taps">Tap here</button>
<button id="response" onclick="++window.responses;fetch('/fast')">Response</button>
<button id="download" onclick="++window.downloads;location.href='/download'">Download</button>
<button id="popup" onclick="++window.popups;window.open('/popup','_blank')">Popup</button>
<button id="dialog" onclick="++window.dialogs;window.answer=prompt('exact question','default')">Dialog</button>
<button id="timeout" onclick="++window.timeouts">Timeout</button>
<div id="host"></div><script>window.scoped=window.shadows=window.wrong=window.taps=window.responses=window.downloads=window.popups=window.dialogs=window.timeouts=0;
const shadow=document.querySelector('#host').attachShadow({mode:'open'});
shadow.innerHTML='<button onclick="++window.shadows">Shadow Action</button>';</script>'''


class Fixture(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        path = urllib.parse.urlsplit(self.path).path
        if path == '/redirect':
            self.send_response(302)
            self.send_header('Location', '/secret')
            self.send_header('Content-Length', '0')
            self.end_headers()
            return
        data, mime = {
            '/': (HTML, 'text/html'),
            '/frame': ("<button onclick=\"++parent.frameClicks\">Frame Action</button><script>parent.frameClicks=0</script>", 'text/html'),
            '/frame2': ('<button>new child</button>', 'text/html'),
            '/popup': ('<title>Exact Popup</title><p>popup ready</p>', 'text/html'),
            '/secret': (json.dumps({'safe': 'visible', 'nested': {'password': 'synthetic-password-984', 'token': 'synthetic-token-736'}}), 'application/json'),
            '/direct-secret': (json.dumps({'safe': 'visible', 'nested': {'password': 'synthetic-password-984', 'token': 'synthetic-token-736'}}), 'application/json'),
            '/fast': ('{"fast":"exact-value"}', 'application/json'),
            '/download': ('exact-download-bytes', 'application/octet-stream'),
            '/echo': (json.dumps({'header': self.headers.get('X-Roadmap', '')}), 'application/json'),
            '/route': ('network-original', 'text/plain'),
        }.get(path, ('{}', 'application/json'))
        data = data.encode()
        self.send_response(200)
        self.send_header('Content-Type', mime)
        self.send_header('Content-Length', str(len(data)))
        if path == '/download':
            self.send_header('Content-Disposition', 'attachment; filename="roadmap.txt"')
        self.end_headers()
        self.wfile.write(data)


class Qualification:
    def __init__(self, args):
        self.args = args
        self.out = Path(args.output_dir)
        self.out.mkdir(parents=True, exist_ok=True)
        self.chat = 'roadmap-' + uuid.uuid4().hex
        self.seq = 0
        self.sessions = {}
        self.results = {}
        self.default_tabs = set()
        self.not_covered = []
        self.generations = {}
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def http_json(self, path, payload=None):
        self.seq += 1
        stem = self.out / ('%04d' % self.seq)
        data = None if payload is None else json.dumps(payload, ensure_ascii=False).encode()
        if data is not None:
            stem.with_suffix('.request.json').write_bytes(data)
        request = urllib.request.Request(self.args.url.rstrip('/') + path, data=data,
                                         headers={'Content-Type': 'application/json'})
        try:
            with self.http.open(request, timeout=90) as response:
                raw = response.read(12 * 1024 * 1024 + 1)
        except urllib.error.HTTPError as exc:
            stem.with_suffix('.response.raw').write_bytes(exc.read())
            raise
        stem.with_suffix('.response.raw').write_bytes(raw)
        require(len(raw) <= 12 * 1024 * 1024, 'response byte budget exceeded')
        return json.loads(raw)

    def call(self, steps, *, session=None, expected=None, chat=None, failure=False, allow_omitted=False, **options):
        require(0 < len(steps) <= 64, 'batch size')
        lifecycle = all(s['action'] in ('create_session', 'list_sessions', 'close_session') for s in steps)
        request = {'steps': steps, **options} if lifecycle else {
            'steps': steps, 'page_context': 'none', 'network': 'none',
            'attach_screenshot': False, **options}
        if session is not None:
            request['session'] = session
            request['expected_session_id'] = expected if expected is not None else self.sessions[session]
        call_id = 'call_' + uuid.uuid4().hex
        payload = {'messages': [{'role': 'assistant', 'content': '', 'tool_calls': [
            {'id': call_id, 'type': 'function', 'function': {'name': 'chrome',
             'arguments': json.dumps({'request': request}, ensure_ascii=False)}}]}],
            'n_ctx': 131072, 'maxgen': 4096, 'subchat_tool_parameters': {},
            'postprocess_parameters': {}, 'model_name': '', 'chat_id': chat or self.chat}
        response = self.http_json('/v1/tools-execute', payload)
        # tools_ran denotes correction processing, not whether a tool executed.
        try:
            report, _ = text_report(response, call_id)
        except (ValueError, AssertionError, KeyError):
            if failure:
                messages = [m for m in response.get('messages', []) if m.get('role') == 'tool'
                            and m.get('tool_call_id') == call_id]
                require(len(messages) == 1 and 'error' in str(messages[0].get('content', '')).lower(),
                        'expected explicit error, not help or malformed response')
                return None
            raise
        for index, step in enumerate(steps):
            if session is None and step['action'] == 'open_tab':
                data = next((s.get('data') or {} for s in report.get('steps', [])
                             if s.get('step_index') == index), {})
                if data.get('tab_id'):
                    self.default_tabs.add(data['tab_id'])
        generation = report.get('session_generation')
        if generation:
            key = (chat or self.chat, session, expected or self.sessions.get(session))
            require(self.generations.get(key, generation) == generation,
                    'unexpected generation replacement; no action is replayed')
            self.generations[key] = generation
        if failure:
            require(report.get('ok') is False and any(
                s.get('ok') is False and s.get('error') for s in report.get('steps', [])),
                'expected explicit failed step, not help')
        else:
            if allow_omitted:
                completion = report['completion']
                require(report.get('ok') is True and completion['steps_requested'] == len(steps)
                        and completion['steps_reported'] == len(steps)
                        and not any(completion.get(k) for k in ('failed', 'unknown', 'skipped', 'incomplete')),
                        'compacted report lacks a complete successful verdict')
            else:
                check_completion(report, len(steps))
        return report

    def create(self, name):
        report = self.call([{'action': 'create_session', 'name': name}])
        data = report['steps'][0]['data']
        # Lifecycle returns RuntimeInfo; reject absent/invalid UUID rather than guessing.
        info = data.get('session', data)
        sid = info['session_id']
        uuid.UUID(sid)
        self.sessions[name] = sid
        return sid

    def close(self, name):
        sid = self.sessions[name]
        report = self.call([{'action': 'close_session', 'name': name, 'expected_session_id': sid}])
        del self.sessions[name]
        return report

    def oracle(self, name, expression):
        result = self.call([{'action': 'eval', 'expression':
            "(async()=>{if(!(await (" + expression + ")))throw Error('roadmap oracle');return true})()"}], session=name)
        require(result['steps'][0]['data']['value'] is True, 'oracle did not return true')

    def case(self, name, fn):
        prior = set(self.sessions)
        try:
            fn()
            self.results[name] = {'passed': True}
        except Exception:
            self.results[name] = {'passed': False, 'error': traceback.format_exc()}
        finally:
            errors = []
            for owned in set(self.sessions) - prior:
                try:
                    self.close(owned)
                except Exception:
                    errors.append(traceback.format_exc())
            if errors:
                self.results[name].update(passed=False, cleanup_errors=errors)
        (self.out / 'effects.json').write_text(json.dumps(self.results, indent=2), encoding='utf-8')

    def readiness(self):
        deadline = time.monotonic() + 30
        while True:
            try:
                groups = self.http_json('/v1/tools')
                tools = [t for g in groups for t in g['tools'] if t['spec']['name'] == 'chrome']
                require(len(tools) == 1 and tools[0]['enabled'], 'chrome unavailable')
                spec = json.dumps(tools[0]['spec'])
                for feature in ('create_session', 'search_snapshot', 'network_requests', 'expected_session_id'):
                    require(feature in spec, 'candidate schema missing ' + feature)
                return
            except (OSError, urllib.error.URLError):
                if time.monotonic() >= deadline:
                    raise
                time.sleep(.25)

    def fresh(self, prefix):
        name = prefix + '-' + uuid.uuid4().hex[:10]
        self.create(name)
        tab = self.call([{'action': 'open_tab', 'url': self.base}], session=name)['steps'][0]['data']['tab_id']
        return name, tab

    def data(self, name, step, **options):
        report = self.call([step], session=name, allow_omitted=True, **options)
        if not report.get('steps'):
            artifact = report.get('full_report')
            require(artifact and 0 < artifact['bytes'] <= 512 * 1024, 'omitted result has no bounded retained report')
            offset, chunks = 0, []
            for _ in range(80):
                read = self.call([{'action': 'read_artifact', 'id': artifact['id'], 'offset': offset, 'limit': 8191}], session=name)
                data = read['steps'][0]['data']
                chunk = base64.b64decode(data['content_base64'], validate=True)
                require(chunk and data['total_bytes'] == artifact['bytes'], 'invalid retained result chunk')
                chunks.append(chunk)
                offset += len(chunk)
                require(offset <= artifact['bytes'], 'retained result exceeded byte size')
                if data.get('next_offset') is None:
                    require(offset == artifact['bytes'], 'retained result ended early')
                    break
                require(data['next_offset'] == offset, 'retained result cursor mismatch')
            else:
                raise AssertionError('retained result chunk limit')
            raw = b''.join(chunks)
            (self.out / ('retained-result-%04d.json' % self.seq)).write_bytes(raw)
            report = json.loads(raw)
            check_completion(report, 1)
        return report['steps'][0]['data']

    @staticmethod
    def click(identity):
        return {'action': 'click', 'locator': {'by': 'id', 'value': identity}}

    def session_case(self):
        default = self.call([{'action': 'open_tab', 'url': self.base},
                             {'action': 'set_viewport', 'width': 1020, 'height': 680},
                             {'action': 'eval', 'expression': "window.roadmapSentinel='DEFAULT'"}])['steps'][0]['data']['tab_id']
        a, atab = self.fresh('isolation-a')
        b, btab = self.fresh('isolation-b')
        old = self.sessions[a]
        for name, marker, width in ((a, 'A', 910), (b, 'B', 730)):
            self.call([{'action': 'set_viewport', 'width': width, 'height': 640},
                       {'action': 'set_extra_http_headers', 'headers': {'X-Roadmap': marker}},
                       {'action': 'route', 'pattern': '**/route', 'handler': {
                           'type': 'fulfill', 'body': 'route-' + marker, 'content_type': 'text/plain'}},
                       {'action': 'eval', 'expression': "window.roadmapSentinel=" + json.dumps(marker) +
                        ";document.cookie='roadmap=" + marker + "; Path=/';localStorage.setItem('roadmap'," + json.dumps(marker) + ")"}], session=name)
            self.oracle(name, "window.innerWidth===" + str(width) +
                        " && document.cookie.includes('roadmap=" + marker + "') && localStorage.roadmap===" + json.dumps(marker))
            self.oracle(name, "fetch('/echo').then(r=>r.json()).then(x=>x.header===" + json.dumps(marker) + ")")
            self.oracle(name, "fetch('/route').then(r=>r.text()).then(x=>x===" + json.dumps('route-' + marker) + ")")
        self.oracle(a, "roadmapSentinel==='A' && localStorage.roadmap==='A' && document.cookie.includes('roadmap=A') && !document.cookie.includes('roadmap=B') && innerWidth===910")
        self.oracle(a, "fetch('/echo').then(r=>r.json()).then(x=>x.header==='A')")
        self.oracle(a, "fetch('/route').then(r=>r.text()).then(x=>x==='route-A')")
        listed = self.call([{'action': 'list_sessions'}])['steps'][0]['data']['sessions']
        require({self.sessions[a], self.sessions[b]}.issubset({s['session_id'] for s in listed}), 'named inventory missing owned UUIDs')
        self.call([{'action': 'eval', 'expression': "window.roadmapSentinel='CORRUPTED'"}],
                  session=a, target={'type': 'id', 'id': btab}, failure=True)
        self.oracle(b, "window.roadmapSentinel==='B' && localStorage.roadmap==='B' && innerWidth===730")
        self.call([{'action': 'close_session', 'name': b, 'expected_session_id': self.sessions[b]}],
                  chat=self.chat + '-other', failure=True)
        self.call([{'action': 'eval', 'expression': "window.roadmapSentinel='CORRUPTED'"}],
                  session=b, chat=self.chat + '-other', failure=True)
        self.call([{'action': 'eval', 'expression': "window.roadmapSentinel='CORRUPTED'"}],
                  session=b, expected=str(uuid.uuid4()), failure=True)
        self.oracle(b, "window.roadmapSentinel==='B'")
        self.close(a)
        self.oracle(b, "window.roadmapSentinel==='B' && localStorage.roadmap==='B' && document.cookie.includes('roadmap=B') && innerWidth===730")
        self.oracle(b, "fetch('/route').then(r=>r.text()).then(x=>x==='route-B')")
        self.oracle(b, "fetch('/echo').then(r=>r.json()).then(x=>x.header==='B')")
        self.create(a)
        self.call([{'action': 'eval', 'expression': "window.roadmapSentinel='CORRUPTED'"}],
                  session=a, expected=old, failure=True)
        self.call([{'action': 'eval', 'expression': '1'}], session=a,
                  target={'type': 'id', 'id': atab}, failure=True)
        self.call([{'action': 'eval', 'expression': "(async()=>{if(window.roadmapSentinel!=='DEFAULT'||localStorage.roadmap||document.cookie.includes('roadmap=')||innerWidth!==1020)throw Error('default corrupted');if((await fetch('/echo').then(r=>r.json())).header!==''||await fetch('/route').then(r=>r.text())!=='network-original')throw Error('default context leaked');return true})()"}],
                  target={'type': 'id', 'id': default})
        self.oracle(b, "window.roadmapSentinel==='B'")

    def search_case(self):
        name, _ = self.fresh('search')
        snapshot = self.data(name, {'action': 'accessibility_snapshot', 'include_frames': True, 'max_frame_depth': 2})
        require('Scope Alpha' in snapshot['yaml'] and any('Frame Action' in f['yaml'] for f in snapshot['frames']), 'frame snapshot missing real child content')
        def search(pattern, **options):
            return self.data(name, {'action': 'search_snapshot', 'pattern': pattern, 'max_results': 20, **options})
        region = search('Scope Alpha')['matches']
        require(len(region) == 1, 'region must be unique')
        scoped = search('Scoped Save', subtree=region[0]['reference'])['matches']
        require(len(scoped) == 1 and scoped[0]['name'] == 'Scoped Save' and region[0]['reference'] in scoped[0]['ancestor_refs'], 'subtree leaked outside button')
        self.call([{'action': 'click', 'locator': {'by': 'ref', 'value': scoped[0]['reference']}}], session=name)
        shadow = search('^Shadow Action$', regex=True, case_sensitive=True)['matches']
        require(len(shadow) == 1, 'open shadow button missing')
        self.call([{'action': 'click', 'locator': {'by': 'ref', 'value': shadow[0]['reference']}}], session=name)
        found = search('^Frame Action$', regex=True, include_frames=True, max_frame_depth=2)['matches']
        child = [m for m in found if m['frame_id'] != snapshot['root_frame_id']]
        require(len(found) == 2 and len(child) == 1, 'main/child identity distinction missing')
        ref = child[0]['reference']
        scoped_child = search('^Frame Action$', regex=True, subtree=ref, include_frames=True)['matches']
        require(len(scoped_child) == 1 and scoped_child[0]['frame_id'] == child[0]['frame_id'],
                'live child subtree did not retain its frame scope')
        ref = scoped_child[0]['reference']
        self.call([{'action': 'click', 'locator': {'by': 'ref', 'value': ref}}], session=name)
        self.oracle(name, 'frameClicks===1 && wrong===0 && scoped===1 && shadows===1')
        self.call([{'action': 'eval', 'expression': "new Promise(resolve=>{const f=document.querySelector('#child');f.onload=()=>resolve(true);f.src='/frame2'})"}], session=name)
        self.call([{'action': 'click', 'locator': {'by': 'ref', 'value': ref}, 'timeout_ms': 200}], session=name, failure=True)
        self.oracle(name, 'frameClicks===1 && wrong===0')
        bounded = search('button', max_results=1)
        require(len(bounded['matches']) == 1 and bounded['truncated'] is True, 'search bound undisclosed')

    def network_case(self):
        name, main = self.fresh('network')
        self.data(name, {'action': 'eval', 'expression': "fetch('/redirect').then(r=>r.json()).then(()=>true)"})
        listing = self.data(name, {'action': 'network_requests', 'limit': 20, 'url_contains': self.base.rstrip('/'), 'method': 'GET'})
        entries = listing['requests']
        secret = [e for e in entries if e['url'] == self.base + 'secret' and e['status'] == 200]
        redirect = [e for e in entries if e['url'] == self.base + 'redirect' and e['status'] == 302]
        require(len(secret) == len(redirect) == 1, 'redirect chain not retained exactly')
        redirected_id, rid = secret[0]['request_id'], redirect[0]['request_id']
        require(redirected_id != rid and secret[0]['redirect_from'] == self.base + 'redirect', 'redirect identity reused')
        self.call([{'action': 'network_request', 'request_id': redirected_id, 'include_body': True}], session=name, failure=True)
        self.data(name, {'action': 'eval', 'expression': "fetch('/direct-secret').then(r=>r.json()).then(()=>true)"})
        direct = self.data(name, {'action': 'network_requests', 'limit': 20, 'url_contains': '/direct-secret', 'method': 'GET', 'status': 200})['requests']
        require(len(direct) == 1, 'direct JSON request not retained exactly')
        sid = direct[0]['request_id']
        body = self.data(name, {'action': 'network_request', 'request_id': sid, 'include_body': True})['body']
        require(isinstance(body, str) and len(body.encode()) <= 65536, 'body must be bounded JSON string')
        decoded = json.loads(body)
        require(decoded == {'safe': 'visible', 'nested': {'password': '[REDACTED]', 'token': '[REDACTED]'}}, 'nested secrets not safely masked')
        require('synthetic-password-984' not in body and 'synthetic-token-736' not in body, 'body secret leak')
        self.call([{'action': 'network_request', 'request_id': rid, 'include_body': True}], session=name, failure=True)
        other = self.data(name, {'action': 'open_tab', 'url': self.base})['tab_id']
        require(other != main, 'second tab not created')
        self.call([{'action': 'network_request', 'request_id': sid, 'include_body': True}], session=name, failure=True)
        second, _ = self.fresh('network-other')
        self.call([{'action': 'network_request', 'request_id': sid, 'include_body': True}], session=second, failure=True)
        self.call([{'action': 'switch_tab', 'tab': {'type': 'id', 'id': main}}], session=name)
        require(self.data(name, {'action': 'network_request', 'request_id': sid})['request']['request_id'] == sid, 'lookup consumed discovery entry')
        if self.args.eviction:
            fence = self.call([
                {'action': 'eval', 'expression': "(async()=>{for(let i=0;i<1001;i++)await fetch('/fast?evict='+i).then(r=>r.text());await fetch('/eviction-fence').then(r=>r.text());return true})()"},
                {'action': 'wait_for_response', 'pattern': '**/eviction-fence', 'method': 'GET', 'status': 200, 'timeout_ms': 10000},
            ], session=name)
            require(fence['steps'][1]['data']['url'] == self.base + 'eviction-fence', 'network monitor did not observe the final eviction fence')
            self.call([{'action': 'network_request', 'request_id': sid}], session=name, failure=True)
            bounded = self.data(name, {'action': 'network_requests', 'limit': 200})
            require(len(bounded['requests']) == 200, 'bounded retained listing incorrect')
        else:
            self.not_covered.append('1000-entry eviction stress (enable --eviction); not a passed eviction oracle')

    def waits_case(self):
        name, main = self.fresh('waits')
        report = self.call([self.click('response'), {'action': 'wait_for_response', 'pattern': '**/fast', 'method': 'GET', 'status': 200, 'timeout_ms': 3000}], session=name)
        entry = report['steps'][1]['data']
        require(entry['url'] == self.base + 'fast' and entry['status'] == 200, 'wrong fast response')
        self.oracle(name, 'responses===1')
        response_body = self.data(name, {'action': 'network_request', 'request_id': entry['request_id'], 'include_body': True})['body']
        require(json.loads(response_body) == {'fast': 'exact-value'}, 'matched fast response body differs')
        report = self.call([self.click('download'), {'action': 'wait_for_download', 'timeout_ms': 3000}], session=name)
        download = report['steps'][1]['data']
        require(download['state'] == 'completed' and download['suggested_filename'] == 'roadmap.txt', 'download not completed exactly')
        require(download['received_bytes'] == len(b'exact-download-bytes') and Path(download['local_path']).read_bytes() == b'exact-download-bytes', 'download bytes differ')
        self.oracle(name, 'downloads===1')
        report = self.call([{'action': 'wait_for_popup', 'timeout_ms': 3000}, self.click('popup')], session=name)
        popup = report['steps'][0]['data']['tab_id']
        require(popup != main, 'popup reused parent identity')
        self.call([{'action': 'expect_poll', 'expression': 'document.title', 'matcher': 'equals', 'expected': 'Exact Popup', 'timeout_ms': 3000}], session=name)
        self.call([{'action': 'close_tab', 'tab': {'type': 'id', 'id': popup}}, {'action': 'switch_tab', 'tab': {'type': 'id', 'id': main}}], session=name)
        self.oracle(name, 'popups===1')
        report = self.call([{'action': 'handle_dialog', 'accept': True, 'prompt_text': 'exact answer'}, self.click('dialog')], session=name)
        require(any(d['type'] == 'prompt' and d['message'] == 'exact question' and d['default_value'] == 'default' and d['action'] == 'accepted' and not d['automatic'] for d in report['dialogs']), 'dialog action receipt incorrect')
        self.oracle(name, "dialogs===1 && answer==='exact answer'")
        report = self.call([self.click('timeout'), {'action': 'wait_for_response', 'pattern': '**/never-issued', 'timeout_ms': 150}], session=name, failure=True)
        require(report is not None, 'timeout must return structured executed evidence')
        check_completion(report, 2, failed=1)
        self.oracle(name, 'timeouts===1 && responses===1 && downloads===1 && popups===1 && dialogs===1')

    def preflight_touch_case(self):
        probe = self.chat + '-preflight'
        before = self.http_json('/v1/browser/status', {'chat_id': probe})
        self.call([{'action': 'help', 'invalid_roadmap_field': True}], chat=probe, failure=True)
        after = self.http_json('/v1/browser/status', {'chat_id': probe})
        require(not before.get('connected') and not after.get('connected') and before == after, 'invalid help implicitly launched browser')
        name, _ = self.fresh('touch-clock')
        self.call([{'action': 'set_viewport', 'width': 800, 'height': 600, 'is_mobile': False, 'has_touch': True}], session=name)
        self.call([{'action': 'tap', 'locator': {'by': 'id', 'value': 'tap'}}], session=name, failure=True)
        self.oracle(name, 'taps===0')
        self.call([{'action': 'set_viewport', 'width': 390, 'height': 844, 'is_mobile': True, 'has_touch': True},
                   {'action': 'tap', 'locator': {'by': 'id', 'value': 'tap'}}], session=name)
        self.oracle(name, 'taps===1 && navigator.maxTouchPoints>0')
        self.call([{'action': 'clock_install', 'time': 1700000000000},
                   {'action': 'clock_pause_at', 'time': 1700000000100}, self.click('timeout')], session=name)
        self.oracle(name, 'timeouts===1 && taps===1 && Date.now()===1700000000100')

    def report_case(self):
        name, _ = self.fresh('report')
        value = '雪🐸café-' * 5000
        report = self.call([{'action': 'eval', 'expression': json.dumps(value, ensure_ascii=False)}, self.click('timeout')], session=name)
        require('full_report' in report, 'large structured result not retained')
        artifact = report['full_report']
        total, offset, chunks = artifact['bytes'], 0, []
        require(0 < total <= 512 * 1024, 'full report byte budget')
        for _ in range(80):
            data = self.data(name, {'action': 'read_artifact', 'id': artifact['id'], 'offset': offset, 'limit': 8191})
            chunk = base64.b64decode(data['content_base64'], validate=True)
            require(data['total_bytes'] == total and chunk, 'empty/changed artifact')
            chunks.append(chunk)
            offset += len(chunk)
            require(offset <= total, 'artifact overflow')
            if data.get('next_offset') is None:
                require(offset == total, 'artifact falsely complete')
                break
            require(data['next_offset'] == offset, 'artifact cursor not byte exact')
        else:
            raise AssertionError('artifact chunk bound exhausted')
        raw = b''.join(chunks)
        (self.out / 'full-report.json').write_bytes(raw)
        full = json.loads(raw)
        check_completion(full, 2)
        require(full['steps'][0]['data']['value'] == value and full['steps'][1]['ok'], 'full report omitted or changed results')
        self.oracle(name, 'timeouts===1')

    def run(self, base):
        self.base = base
        self.readiness()
        for name, fn in (('session_isolation_ownership_and_stale_uuid', self.session_case),
                         ('search_frames_shadow_and_stale_ref', self.search_case),
                         ('network_identity_redaction_redirect', self.network_case),
                         ('armed_waits_and_no_replay', self.waits_case),
                         ('help_preflight_touch_clock', self.preflight_touch_case),
                         ('structured_report_limits', self.report_case)):
            self.case(name, fn)


def self_check():
    class HarnessTests(unittest.TestCase):
        def test_fixture_routes_and_shutdown(self):
            server = ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            base = 'http://127.0.0.1:%d/' % server.server_port
            try:
                for path, expected in (('', b'Shadow Action'), ('frame', b'parent.frameClicks=0'),
                                       ('download', b'exact-download-bytes'), ('fast', b'exact-value'),
                                       ('redirect', b'synthetic-password-984')):
                    with opener.open(base + path, timeout=2) as response:
                        self.assertIn(expected, response.read())
                request = urllib.request.Request(base + 'echo', headers={'X-Roadmap': 'SELF'})
                with opener.open(request, timeout=2) as response:
                    self.assertEqual(json.load(response), {'header': 'SELF'})
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=2)
            self.assertFalse(thread.is_alive())

        def test_valid_wrong_target_and_no_replay(self):
            with tempfile.TemporaryDirectory() as directory:
                runner = Qualification(argparse.Namespace(output_dir=directory, url='http://127.0.0.1:18991'))
                runner.sessions['a'] = str(uuid.uuid4())
                captured = []
                def transport(path, payload):
                    captured.append(payload)
                    call = payload['messages'][0]['tool_calls'][0]
                    return {'tools_ran': False, 'messages': [{'role': 'tool', 'tool_call_id': call['id'],
                        'content': json.dumps({'tool': 'chrome', 'ok': False, 'steps': [
                            {'step_index': 0, 'ok': False, 'error': 'Target not owned'}]})}]}
                runner.http_json = transport
                runner.call([{'action': 'eval', 'expression': "window.sentinel='BAD'"}],
                            session='a', target={'type': 'id', 'id': 'foreign-tab'}, failure=True)
                self.assertEqual(len(captured), 1)
                request = json.loads(captured[0]['messages'][0]['tool_calls'][0]['function']['arguments'])['request']
                self.assertEqual(request['target'], {'type': 'id', 'id': 'foreign-tab'})
                self.assertEqual(request['steps'], [{'action': 'eval', 'expression': "window.sentinel='BAD'"}])
                self.assertEqual(request['expected_session_id'], runner.sessions['a'])
                runner.call([{'action': 'close_session', 'name': 'a', 'expected_session_id': runner.sessions['a']}], failure=True)
                lifecycle_request = json.loads(captured[-1]['messages'][0]['tool_calls'][0]['function']['arguments'])['request']
                self.assertEqual(set(lifecycle_request), {'steps'})

        def test_failure_cannot_be_help_or_success(self):
            with tempfile.TemporaryDirectory() as directory:
                runner = Qualification(argparse.Namespace(output_dir=directory, url='http://127.0.0.1:18991'))
                def transport(path, payload):
                    call_id = payload['messages'][0]['tool_calls'][0]['id']
                    return {'messages': [{'role': 'tool', 'tool_call_id': call_id,
                        'content': json.dumps({'tool': 'chrome', 'ok': True, 'steps': []})}]}
                runner.http_json = transport
                with self.assertRaises(AssertionError):
                    runner.call([{'action': 'help'}], failure=True)

        def test_case_failure_and_owned_cleanup(self):
            with tempfile.TemporaryDirectory() as directory:
                runner = Qualification(argparse.Namespace(output_dir=directory))
                closed = []
                def close(name):
                    closed.append(name)
                    del runner.sessions[name]
                runner.close = close
                def fail():
                    runner.sessions['owned'] = 'synthetic'
                    raise AssertionError('intentional')
                runner.case('failure', fail)
                self.assertFalse(runner.results['failure']['passed'])
                self.assertEqual(closed, ['owned'])
                self.assertTrue((Path(directory) / 'effects.json').is_file())
    result = unittest.TextTestRunner().run(unittest.defaultTestLoader.loadTestsFromTestCase(HarnessTests))
    return 0 if result.wasSuccessful() else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--url')
    parser.add_argument('--output-dir')
    parser.add_argument('--eviction', action='store_true', help='also execute bounded 1001-request eviction stress')
    parser.add_argument('--self-check', action='store_true', help='fixture/parser tests only; does not qualify an engine')
    args = parser.parse_args()
    if args.self_check:
        return self_check()
    require(args.output_dir, '--output-dir required for retained receipts')
    parsed = urllib.parse.urlsplit(args.url or '')
    require(parsed.scheme == 'http' and parsed.hostname in ('127.0.0.1', 'localhost', '::1')
            and parsed.port not in (None, 8001, 18990) and not parsed.username and not parsed.password
            and not parsed.path.strip('/') and not parsed.query and not parsed.fragment,
            'explicit isolated loopback worker URL required; :8001 forbidden')
    runner = Qualification(args)
    error = None
    cleanup = []
    server = ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        runner.run('http://127.0.0.1:%d/' % server.server_port)
    except Exception:
        error = traceback.format_exc()
    finally:
        for name in list(runner.sessions):
            try:
                runner.close(name)
            except Exception:
                cleanup.append(traceback.format_exc())
        for tab in list(runner.default_tabs):
            try:
                runner.call([{'action': 'close_tab', 'tab': {'type': 'id', 'id': tab}}])
                runner.default_tabs.remove(tab)
            except Exception:
                cleanup.append(traceback.format_exc())
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)
        if thread.is_alive():
            cleanup.append('fixture thread did not stop')
        passed = error is None and not cleanup and len(runner.results) == 6 and all(
            result['passed'] for result in runner.results.values())
        summary = {'passed': passed, 'chat_id': runner.chat, 'cases': runner.results,
                   'not_covered': runner.not_covered,
                   'error': error, 'cleanup_errors': cleanup}
        (runner.out / 'summary.json').write_text(json.dumps(summary, indent=2), encoding='utf-8')
        print(json.dumps({'passed': passed, 'output_dir': str(runner.out), 'error': error}))
    return 0 if passed else 1


if __name__ == '__main__':
    raise SystemExit(main())

#!/usr/bin/env python3
"""Opt-in actual-model worker barrier/cancel/restart smoke on dedicated :8006.

Uses public model tools and HTTP, never DB seeding or product test hooks. The
PTY release token, not elapsed time, orders A inspection -> B activation -> reply.
Run only with an authorized configured model; no existing server is reused.
"""
import argparse
import asyncio
import hashlib
import json
import shlex
import shutil
import socket
import subprocess
import tempfile
import time
import traceback
import uuid
from pathlib import Path
from types import SimpleNamespace

import aiohttp
from test_review_tool_smoke import Runner, calls, fixture, source_identity
from test_review_lifecycle_smoke import review_metadata

URL = 'http://127.0.0.1:8006'
TERMINAL = {'completed', 'failed', 'cancelled', 'interrupted', 'closed'}


class ReceiptRunner(Runner):
    async def request(self, method, path, body=None):
        async with self.http.request(method, self.o.url + '/v1' + path, json=body,
                                     timeout=aiohttp.ClientTimeout(total=30)) as response:
            data = await response.json(content_type=None)
            assert response.status < 300, f'{method} {path}: HTTP {response.status}: {str(data)[:500]}'
            return data

    async def turn(self, prompt, scope='single'):
        self.deadline = time.monotonic() + self.o.seconds
        await super().turn(prompt, scope)
        async def completed_turn():
            snapshot = await self.snapshot()
            self.save(snapshot)
            messages = snapshot.get('messages', [])
            matching = [index for index, message in enumerate(messages)
                        if message.get('role') == 'user' and message.get('content') == prompt]
            if not matching:
                return None
            tail = messages[matching[-1] + 1:]
            assistant = [message for message in tail if message.get('role') == 'assistant']
            if snapshot.get('runtime', {}).get('state') not in ('idle', 'completed') or not assistant:
                return None
            if assistant[-1].get('tool_calls'):
                results = {message.get('tool_call_id') for message in tail if message.get('role') == 'tool'}
                if any(call['id'] not in results for call in assistant[-1]['tool_calls']):
                    return None
            return snapshot
        return await until('requested model turn completed', completed_turn, self.o.seconds)

    def save(self, snap):
        # Retain identities/counts only, not model text, credentials or auth data.
        safe = {'chat_id': self.chat, 'state': snap.get('runtime', {}).get('state'),
                'calls': [{'id': c['id'], 'name': c['name'], 'success': c['success']}
                          for c in calls(snap)]}
        (self.o.logs / (self.chat + '.json')).write_text(json.dumps(safe, indent=2))


class Engine:
    def __init__(self, binary, root):
        self.binary, self.root, self.process = binary, root, None

    def start(self):
        with socket.socket() as sock:
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            sock.bind(('127.0.0.1', 8006))
        self.process = subprocess.Popen(
            [str(self.binary), '--http-port', '8006', '--workspace-folder', str(self.root),
             '--no-scheduler', '--logs-stderr'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            assert self.process.poll() is None, 'owned engine exited before readiness'
            try:
                with socket.create_connection(('127.0.0.1', 8006), timeout=.3):
                    return
            except OSError:
                time.sleep(.1)
        raise AssertionError('owned engine readiness timeout')

    def stop(self):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)


async def until(label, callback, seconds):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = await callback()
        if result:
            return result
        await asyncio.sleep(.5)
    raise AssertionError(label + ': finite assertion timeout')


async def exercise(args, root, info, engine, report):
    options = SimpleNamespace(url=URL, model=args.model, logs=args.evidence,
                              max_turns=6, seconds=args.seconds)
    owned = []
    async with aiohttp.ClientSession(read_bufsize=16 * 1024 * 1024,
                                     connector=aiohttp.TCPConnector(force_close=True)) as http:
        parent = ReceiptRunner(options, http)
        parent.chat = 'worker-smoke-' + uuid.uuid4().hex
        parent.deadline = time.monotonic() + args.seconds
        report['chat_id'] = parent.chat
        async def get_review():
            return await parent.request('GET', '/reviews/by-chat/' + parent.chat)
        async def agents():
            rows = await parent.request('GET', '/background-agents?chat_id=' + parent.chat)
            assert isinstance(rows, list), 'background agent list shape changed'
            return [{**row, 'agent_id': row['agentId'], 'child_chat_id': row.get('childChatId')}
                    for row in rows]
        async def child_snapshot(child_id):
            child = ReceiptRunner(options, http)
            child.chat = child_id
            snap = await child.snapshot()
            child.save(snap)
            return snap
        try:
            initialized = await parent.request('POST', '/lsp-initialize', {'project_roots': [root.as_uri()]})
            assert initialized['success'] == 1 and initialized['files_found'] > 0
            await parent.snapshot()
            await parent.command('set_params', patch={'model': args.model, 'mode': 'review',
                'auto_approve_editing_tools': True, 'auto_approve_dangerous_commands': True})
            snap = await parent.turn(
                f'Disposable synthetic fixture only. Activate set_review_context repository {root}, '
                f'base_ref {info["base"]}, candidate_ref {info["one"]}. Actually call tool then stop. '
                'No network, publication, merge or source edits.')
            assert any(c['name'] == 'set_review_context' and c['success'] for c in calls(snap)), 'model did not activate synthetic candidate'
            review = await get_review()
            path = '/reviews/' + review['id']
            candidate_a = review['current_candidate_id']
            anchor = await parent.request('POST', path + '/anchors', {'candidate_id': candidate_a,
                'path': 'calculator.py', 'side': 'head', 'start_line': 1, 'end_line': 2})
            await parent.request('POST', path + '/comments', {'expected_revision': review['revision'],
                'request_key': uuid.uuid4().hex, 'anchor': anchor, 'body': 'Synthetic worker discussion'})
            review = await get_review()
            thread_id = review['threads'][-1]['id']
            catalog = await parent.request('GET', '/tools')
            names = {'subagent', 'review_get', 'review_reply', 'process_start', 'wait', 'shell', 'create_textdoc'}
            specs = [t['spec'] for group in catalog for t in group['tools'] if t['spec']['name'] in names]
            assert {'subagent', 'review_get', 'review_reply', 'process_start', 'wait'} <= {s['name'] for s in specs}

            async def launch(label):
                previous = {a['agent_id'] for a in await agents()}
                token = 'release-' + uuid.uuid4().hex
                wip = 'worker-wip-' + label + '.txt'
                release = 'worker-release-' + label + '.txt'
                # select is bounded; no elapsed-time path can produce a successful release.
                code = ("import pathlib,select,sys; print('BARRIER_READY',flush=True); "
                        "ready=select.select([sys.stdin],[],[],240)[0]; "
                        "line=sys.stdin.readline().strip() if ready else ''; "
                        f"assert line=={token!r}, 'barrier not released'; "
                        f"pathlib.Path({release!r}).write_text(line); print('BARRIER_RELEASED',flush=True)")
                command = 'python3 -u -c ' + shlex.quote(code)
                task = (f'Synthetic controlled worker {label}. No network, merge, publication or original source edits. '
                    'FIRST review_get immutable calculator.py head lines 1-2 for your pinned candidate. '
                    f'Then write {wip} with exact content SYNTHETIC_WIP_{label} in your accepted registered investigation checkout only. '
                    f'Then process_start command {command!r}, tty=true, workdir your accepted checkout. '
                    'Then use wait target kind process with returned process_id. Stay pending until process exits. '
                    f'ONLY after successful release, review_get summary for current revision then review_reply thread_id {thread_id} '
                    f'body SYNTHETIC_LATE_{label}; use latest expected_revision and unique request_key. '
                    'Reply must retain your original pinned candidate. Stop after successful reply. '
                    'Do not release yourself or invoke process_write_stdin. Advertised relevant schemas: ' + json.dumps(specs))
                launch_request = {'task': task, 'expected_result': 'Pinned synthetic review reply after controlled release',
                                  'model_name': args.model, 'worktree': 'inherit',
                                  'tools': 'review_get,review_reply,process_start,wait,shell,create_textdoc'}
                launched = await parent.turn('Call subagent now with these exact arguments: ' + json.dumps(launch_request)
                    + '. Return after launch, do not call review_get yourself or wait for child.')
                if not any(c['name'] == 'subagent' for c in calls(launched)):
                    report['launch_assessment'] = [m.get('content', '') for m in launched.get('messages', [])
                                                   if m.get('role') == 'assistant'][-2:]
                    raise AssertionError('model did not call subagent')
                async def find():
                    fresh = [a for a in await agents() if a['agent_id'] not in previous]
                    assert len(fresh) <= 1, 'unexpected extra worker'
                    return fresh[0] if fresh else None
                agent = await until('worker accepted', find, 30)
                owned.append(agent)
                async def pending():
                    rows = [a for a in await agents() if a['agent_id'] == agent['agent_id']]
                    assert rows and rows[0]['status'] not in TERMINAL, 'worker ended before controlled barrier'
                    child_id = rows[0].get('child_chat_id')
                    if not child_id:
                        return None
                    cs = await child_snapshot(child_id)
                    actual = calls(cs)
                    reads = [c for c in actual if c['name'] == 'review_get' and c['success']
                             and c['args'].get('path') == 'calculator.py'
                             and c['value'].get('candidate_id') == (await get_review())['current_candidate_id']]
                    starts = [c for c in actual if c['name'] == 'process_start'
                              and command == c['args'].get('command')
                              and 'Process started' in c['result_text']]
                    waits = [c for c in actual if c['name'] == 'wait' and c['args'].get('target', {}).get('kind') == 'process']
                    if not (reads and starts and waits):
                        return None
                    pid = waits[-1]['args']['target']['process_id']
                    process = await parent.request('GET', f'/exec/{pid}/read?chat_id={child_id}')
                    output = ''.join(c['text'] for c in process.get('chunks', []))
                    assert process['status'] == 'running', 'barrier process is not pending'
                    if 'BARRIER_READY' not in output or 'BARRIER_RELEASED' in output:
                        return None
                    assert not waits[-1]['success'], 'wait already completed before release'
                    wt = cs['thread']['worktree']
                    checkout = Path(wt['root'])
                    assert checkout.resolve() != root.resolve() and wt['kind'] == 'review'
                    assert (checkout / wip).read_text().strip() == 'SYNTHETIC_WIP_' + label, 'worker did not retain the requested WIP marker'
                    assert not (checkout / release).exists()
                    return {**rows[0], 'process_id': pid, 'checkout': checkout, 'wip': wip,
                            'release': release, 'token': token, 'read_call_id': reads[-1]['id'],
                            'wait_call_id': waits[-1]['id']}
                return await until('inspected + genuine pending barrier', pending, min(args.seconds, 180))

            late = await launch('late')
            report['checks']['pending_A'] = {k: late[k] for k in ('agent_id', 'process_id', 'read_call_id', 'wait_call_id')}
            review = await get_review()
            update = {'repository': str(root), 'base_ref': info['base'], 'candidate_ref': info['two'],
                'review_id': review['id'], 'expected_revision': review['revision'], 'request_key': uuid.uuid4().hex}
            await parent.turn('Call set_review_context exactly once with ' + json.dumps(update) + '. Stop; do not interact with workers.')
            updated = await get_review()
            candidate_b = updated['current_candidate_id']
            assert candidate_b != candidate_a
            assert not (late['checkout'] / late['release']).exists()
            await parent.request('POST', f'/exec/{late["process_id"]}/stdin',
                                 {'chat_id': late['child_chat_id'], 'chars': late['token'] + '\n'})
            async def replied():
                current = await get_review()
                found = [c for t in current['threads'] for c in t['contributions']
                         if c.get('body') == 'SYNTHETIC_LATE_late']
                if not found:
                    return None
                assert len(found) == 1
                item = found[0]
                assert item['context']['candidate_id'] == candidate_a
                assert item['context']['applicability'] == 'historical'
                run = next(r for r in current['runs'] if r['agent_id'] == late['agent_id'])
                assert run['candidate_id'] == candidate_a and item['context']['run_id'] == run['id']
                assert current['current_candidate_id'] == candidate_b
                return {'contribution_id': item['id'], 'run_id': run['id'], 'candidate_id': candidate_a,
                        'current_candidate_id': candidate_b, 'applicability': 'historical'}
            report['checks']['late_result'] = await until('late historical contribution', replied, args.seconds)
            cancelled = await launch('cancel')
            before_cancel = await get_review()
            before_focus = (await parent.snapshot())['thread']['worktree']
            wip_bytes = (cancelled['checkout'] / cancelled['wip']).read_bytes()
            await parent.request('POST', f'/background-agents/{cancelled["agent_id"]}/cancel',
                                 {'chat_id': parent.chat, 'subtree': True, 'reason': 'synthetic pending cancellation'})
            async def stopped():
                rows = [a for a in await agents() if a['agent_id'] == cancelled['agent_id']]
                return rows[0] if rows and rows[0]['status'] == 'cancelled' else None
            await until('cancel pending worker', stopped, 30)
            after_cancel = await get_review()
            assert after_cancel['current_candidate_id'] == candidate_b
            assert (await parent.snapshot())['thread']['worktree'] == before_focus
            assert (cancelled['checkout'] / cancelled['wip']).read_bytes() == wip_bytes
            assert not (cancelled['checkout'] / cancelled['release']).exists()
            assert not any(c.get('body') == 'SYNTHETIC_LATE_cancel' for t in after_cancel['threads'] for c in t['contributions'])
            assert all(m in after_cancel['materials'] for m in before_cancel['materials'])
            report['checks']['cancel_pending'] = {'agent_id': cancelled['agent_id'], 'wip_preserved': True,
                'focus_unchanged': True, 'candidate_id': candidate_b, 'materials_preserved': True}
            process = await parent.request('GET', f'/exec/{cancelled["process_id"]}/read?chat_id={cancelled["child_chat_id"]}')
            if process['status'] == 'running':
                await parent.request('POST', f'/exec/{cancelled["process_id"]}/kill?chat_id={cancelled["child_chat_id"]}', {})
            latest = await get_review()
            body = 'SYNTHETIC_RESTART_' + uuid.uuid4().hex
            payload = {'expected_revision': latest['revision'], 'request_key': uuid.uuid4().hex,
                       'thread_id': thread_id, 'body': body}
            await parent.request('POST', path + '/comments', payload)  # discard committed ACK
            canonical = await get_review()
            outbox_before = await parent.request('GET', path + '/delivery-status')
            engine.stop()
            engine.start()
            initialized = await parent.request('POST', '/lsp-initialize', {'project_roots': [root.as_uri()]})
            assert initialized['success'] == 1
            # Restore persisted trusted chat through normal subscription, not DB mutation.
            await parent.snapshot()
            recovered = await get_review()
            assert recovered['threads'] == canonical['threads']
            assert recovered['current_candidate_id'] == canonical['current_candidate_id']
            await parent.request('POST', path + '/comments', payload)
            replayed = await get_review()
            found = [c for t in replayed['threads'] for c in t['contributions'] if c.get('body') == body]
            assert len(found) == 1
            cid = found[0]['id']
            async def delivered_once():
                messages = (await parent.snapshot()).get('messages', [])
                found = [m for m in messages if review_metadata(m).get('contribution_id') == cid]
                assert len(found) <= 1, 'restart duplicated delivery'
                return len(found) == 1
            await until('restart delivery', delivered_once, 30)
            outbox_after = await parent.request('GET', path + '/delivery-status')
            report['checks']['restart_lost_ack'] = {'contribution_id': cid, 'canonical_count': 1,
                'delivered_count': 1, 'outbox_before_count': len(outbox_before),
                'outbox_after_count': len(outbox_after), 'ack_simulation': 'committed response discarded'}
        finally:
            for agent in owned:
                try:
                    await parent.request('POST', f'/background-agents/{agent["agent_id"]}/cancel',
                                         {'chat_id': parent.chat, 'subtree': True})
                except Exception:
                    pass  # engine shutdown below also terminates owned process registry
            try:
                await parent.command('abort')
            except Exception:
                pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--model', default='openai_codex_personal/gpt-6.1-sol')
    parser.add_argument('--seconds', type=int, default=300, choices=range(30, 601))
    args = parser.parse_args()
    args.evidence = args.evidence.resolve()
    args.evidence.mkdir(parents=True, exist_ok=True, mode=0o700)
    pinned = args.evidence / 'refact-lsp-pinned'
    assert args.binary.resolve() != pinned, 'supply original binary, not the evidence copy'
    shutil.copy2(args.binary.resolve(strict=True), pinned)
    hasher = hashlib.sha256()
    with pinned.open('rb') as binary_file:
        for chunk in iter(lambda: binary_file.read(1024 * 1024), b''):
            hasher.update(chunk)
    digest = hasher.hexdigest()
    report = {'checks': {}, 'error': None, 'binary_sha256': digest,
              'scope': 'disposable synthetic fixture; actual worker/model; no user consent claim'}
    with tempfile.TemporaryDirectory(prefix='review-worker-') as temp:
        root = Path(temp) / 'workspace'
        info = fixture(root)
        before = source_identity(root)
        engine = Engine(pinned, root)
        try:
            engine.start()
            asyncio.run(asyncio.wait_for(exercise(args, root, info, engine, report), timeout=args.seconds * 4))
        except Exception as exc:
            report['error'] = {'type': type(exc).__name__, 'message': str(exc)[:1000], 'traceback': traceback.format_exc()}
        finally:
            engine.stop()
            report['source_unchanged'] = source_identity(root) == before
            if not report['source_unchanged']:
                report['error'] = {'type': 'SourceMutation'}
    (args.evidence / 'results.json').write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2), flush=True)
    raise SystemExit(1 if report['error'] else 0)


if __name__ == '__main__':
    main()

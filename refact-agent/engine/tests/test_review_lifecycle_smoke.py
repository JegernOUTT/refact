#!/usr/bin/env python3
"""Opt-in synthetic live-engine lifecycle smoke. Never points at an existing server.

Starts a dedicated checkpoint binary on loopback :8006 with one disposable git
workspace, creates a new synthetic chat via Runner, and writes redacted receipts.
No credential files, existing chats, DB writes, network remotes, or rebuilds.
"""
import argparse
import asyncio
import json
import socket
import subprocess
import tempfile
import time
import traceback
import uuid
from pathlib import Path
from types import SimpleNamespace

import aiohttp
from test_review_tool_smoke import Runner, TOOLS, calls, fixture, source_identity

URL = 'http://127.0.0.1:8006'


async def request(session, method, path, body=None):
    async with session.request(method, URL + '/v1' + path, json=body,
                               timeout=aiohttp.ClientTimeout(total=30)) as response:
        return response.status, await response.json(content_type=None)


def review_metadata(message):
    # ChatMessage.extra is serde(flatten) on the public SSE wire. Accept the
    # nested in-memory shape too, but never use role/content as an identity fallback.
    value = message.get('review')
    return value if value is not None else (message.get('extra') or {}).get('review', {})


async def run(args, root, info):
    report = {'scope': 'synthetic fixture only; no real PR or user consent',
              'fixture': str(root), 'checks': {}, 'blockers': [], 'error': None}
    options = SimpleNamespace(url=URL, model=args.model, logs=args.evidence,
                              max_turns=3, seconds=args.seconds)
    async with aiohttp.ClientSession(read_bufsize=16 * 1024 * 1024) as session:
        runner = Runner(options, session)
        try:
            status, init = await request(session, 'POST', '/lsp-initialize',
                                         {'project_roots': [root.as_uri()]})
            assert status == 200 and init['success'] == 1 and init['files_found'] > 0, (status, init)
            report['checks']['workspace_initialize'] = {'status': status, 'files_found': init['files_found'], 'sent_roots': [root.as_uri()]}
            runner.chat = 'review-lifecycle-' + uuid.uuid4().hex
            report['chat_id'] = runner.chat
            runner.deadline = time.monotonic() + args.seconds
            await runner.snapshot()
            await runner.command('set_params', patch={'model': args.model, 'mode': 'review',
                                  'auto_approve_editing_tools': True,
                                  'auto_approve_dangerous_commands': False})
            snap = await runner.turn(
                f"Disposable synthetic fixture only. Do not publish, fetch, push, download, "
                f"or alter repository files. Use set_review_context to activate comparison "
                f"repository {root}, base {info['base']}, candidate {info['one']}. "
                "Actually call the tool, then stop.")
            actual = [c for c in calls(snap) if c['name'] == 'set_review_context' and c['success']]
            assert actual, 'model did not successfully activate synthetic review'
            review = await runner.request('GET', f'/reviews/by-chat/{runner.chat}')
            assert review and review['current_candidate_id']
            rid = review['id']
            path = '/reviews/' + rid
            status, anchor = await request(session, 'POST', path + '/anchors', {
                'candidate_id': review['current_candidate_id'], 'path': 'calculator.py',
                'side': 'head', 'start_line': 1, 'end_line': 2})
            assert status == 200, (status, anchor)
            draft = 'SYNTHETIC lost acknowledgment comment ' + uuid.uuid4().hex
            payload = {'expected_revision': review['revision'], 'request_key': 'lost-ack-' + uuid.uuid4().hex,
                       'anchor': anchor, 'body': draft}
            # Read the server-committed response, then discard it at the client boundary.
            # This models a lost ACK after commit, not an actual transport drop.
            status, committed = await request(session, 'POST', path + '/comments', payload)
            assert status == 200, (status, committed)
            del committed
            status, replay = await request(session, 'POST', path + '/comments', payload)
            assert status == 200, (status, replay)
            status, after = await request(session, 'GET', path)
            assert status == 200
            contributions = [c for t in after['threads'] for c in t.get('contributions', []) if c.get('body') == draft]
            assert len(contributions) == 1, f'expected exactly one canonical contribution, got {len(contributions)}'
            assert replay['revision'] == after['revision'], 'replay changed revision'
            status, delivery = await request(session, 'GET', path + '/delivery-status')
            assert status == 200 and isinstance(delivery, list)
            snap = await runner.snapshot()
            contribution_id = contributions[0]['id']
            delivered = [m for m in snap.get('messages', [])
                         if review_metadata(m).get('contribution_id') == contribution_id]
            assert len(delivered) == 1, f'expected exactly one delivered message for contribution {contribution_id}, got {len(delivered)}'
            assert delivered[0].get('role') == 'user' and delivered[0].get('content') == draft
            report['checks']['lost_ack_exact_retry'] = {'status': 200, 'contribution_count': len(contributions),
                'contribution_id': contribution_id, 'delivered_message_count': len(delivered),
                'revision': after['revision'], 'pending_delivery_count': len(delivery), 'simulation': 'response discarded after success'}
            stale_draft = 'SYNTHETIC stale draft ' + uuid.uuid4().hex
            stale = {**payload, 'request_key': 'stale-' + uuid.uuid4().hex, 'body': stale_draft}
            status, failure = await request(session, 'POST', path + '/comments', stale)
            assert status == 409, (status, failure)
            _, final = await request(session, 'GET', path)
            assert not any(c.get('body') == stale_draft for t in final['threads'] for c in t.get('contributions', []))
            assert stale['body'] == stale_draft, 'client draft changed'
            assert final['revision'] == after['revision']
            report['checks']['stale_cas'] = {'status': status, 'client_draft_preserved': True,
                                              'canonical_duplicates': 0, 'detail': failure}
            if args.refresh:
                # Only normal local set_review_context recapture semantics are tested;
                # this is not Graphite stack_refresh or a remote fetch guarantee.
                for label, candidate_ref in [('noop', info['one']),
                                             ('failed', 'synthetic-missing-' + uuid.uuid4().hex)]:
                    before_refresh = await runner.request('GET', path)
                    before_snap = await runner.snapshot()
                    before_ids = {c['id'] for c in calls(before_snap)}
                    context_args = {'repository': str(root), 'base_ref': info['base'],
                                    'candidate_ref': candidate_ref, 'review_id': rid,
                                    'expected_revision': before_refresh['revision'],
                                    'target_reference': before_refresh['target_reference'],
                                    'request_key': 'refresh-' + uuid.uuid4().hex}
                    snap = await runner.turn(
                        'Synthetic lifecycle test. Call set_review_context exactly once with '
                        + json.dumps(context_args) + '. Do not retry or modify anything else. '
                        'For a missing ref report the error and stop; do not repair the ref.')
                    invoked = [c for c in calls(snap) if c['id'] not in before_ids
                               and c['name'] == 'set_review_context']
                    assert len(invoked) == 1, f'{label}: expected one actual recapture call'
                    assert all(invoked[0]['args'].get(k) == v for k, v in context_args.items()), label
                    assert invoked[0]['success'] == (label == 'noop'), label
                    if label == 'failed':
                        assert candidate_ref in invoked[0]['result_text'], 'missing-ref failure not identified'
                    refreshed = await runner.request('GET', path)
                    assert refreshed == before_refresh, f'{label}: canonical review/history changed'
                    refreshed_snap = await runner.snapshot()
                    assert refreshed_snap.get('thread', {}).get('worktree') == before_snap.get('thread', {}).get('worktree'), label
                    report['checks']['local_recapture_' + label] = {
                        'actual_tool_calls': len(invoked), 'tool_success': invoked[0]['success'],
                        'entire_review_unchanged': True, 'candidate_id': refreshed['current_candidate_id'],
                        'candidate_count': len(refreshed['candidates']), 'revision': refreshed['revision']}
            if args.historical:
                catalog = await runner.request('GET', '/tools')
                specs = [t['spec'] for group in catalog for t in group['tools'] if t['spec']['name'] in TOOLS]
                historical = Runner(options, session)
                try:
                    await asyncio.wait_for(historical.scenario('historical', info, specs), timeout=args.seconds)
                    report['checks']['historical_exact_refusals'] = {'passed': True, 'chat_id': historical.chat}
                except Exception as exc:
                    report['checks']['historical_exact_refusals'] = {'passed': False, 'chat_id': historical.chat,
                                                                       'error': f'{type(exc).__name__}: {exc}'}
                    report['error'] = report['checks']['historical_exact_refusals']
                finally:
                    if historical.chat:
                        await historical.command('abort')
            report['blockers'].extend([
                'Draft preservation here is client payload preservation, not browser editor state.',
                'No controlled pending actual reviewer or release barrier: late worker and cancellation/shutdown delivery not tested.',
                *([] if args.refresh else ['No-op and failed local recapture preservation not requested.']),
                'Graphite stack refresh preservation and real process restart not exercised.',
                *([] if args.historical else ['Historical exact refusal not requested.'])
            ])
        except Exception as exc:
            report['error'] = {'type': type(exc).__name__, 'message': str(exc), 'traceback': traceback.format_exc()}
        finally:
            if runner.chat:
                try:
                    await runner.command('abort')
                except Exception:
                    pass
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--model', default='openai_codex_personal/gpt-6.1-sol')
    parser.add_argument('--seconds', type=int, default=300)
    parser.add_argument('--historical', action='store_true', help='also run real-model historical stack scenario')
    parser.add_argument('--refresh', action='store_true', help='also test actual no-op and failed local recapture tools')
    args = parser.parse_args()
    args.binary = args.binary.resolve(strict=True)
    args.evidence = args.evidence.resolve()
    args.evidence.mkdir(parents=True, exist_ok=True, mode=0o700)
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 8006))  # Refuse an occupied port; never use an existing server.
    with tempfile.TemporaryDirectory(prefix='review-lifecycle-') as temp:
        root = Path(temp) / 'workspace'
        info = fixture(root)
        before = source_identity(root)
        with subprocess.Popen([str(args.binary), '--http-port', '8006', '--workspace-folder', str(root),
                               '--no-scheduler', '--logs-stderr'], stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL) as server:
            try:
                deadline = time.monotonic() + 30
                while True:
                    if server.poll() is not None:
                        raise RuntimeError(f'engine exited before readiness: {server.returncode}')
                    try:
                        with socket.create_connection(('127.0.0.1', 8006), timeout=1):
                            break
                    except OSError:
                        if time.monotonic() >= deadline:
                            raise RuntimeError('engine did not listen on dedicated port')
                        time.sleep(.2)
                report = asyncio.run(run(args, root, info))
                report['source_unchanged'] = source_identity(root) == before
                if not report['source_unchanged']:
                    report['error'] = {'type': 'SourceMutation', 'message': 'synthetic source changed'}
            finally:
                server.terminate()
                try:
                    server.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()
    (args.evidence / 'results.json').write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2), flush=True)
    if report['error']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()

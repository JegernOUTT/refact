"""Disposable loopback-only engine infrastructure; never attach to a live worker."""
import argparse
import asyncio
import json
import hashlib
import shutil
import sys
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import uuid

from aiohttp import web
import aiohttp

ENGINE_PORT = 8017
PROVIDER_PORT = 8018
MODEL = "localstub/foreground-smoke"
MARKER = "TOOL_JOB_SMOKE_NATIVE_RESULT"
CMDLINE_NAME = "cmdline_smoke"
SECOND_PROMPT = "Second human turn while native job is pending."
PHASE_TIMEOUT_SECONDS = 20
RECEIPT_TIMEOUT_SECONDS = 75
BARRIER_TIMEOUT_SECONDS = 120
COMMAND_TIMEOUT_SECONDS = 150


class StubProvider:
    """OpenAI-compatible deterministic tool-calling provider, local only."""
    def __init__(self, barrier: Path, counter: Path, port=0, tool_call=True):
        self.tool_call = tool_call
        self.port = port
        self.barrier, self.counter = barrier, counter
        self.calls = []
        self.emitted = 0
        self.second_seen_pending = False
        self.app = web.Application()
        self.app.router.add_get("/v1/models", self.models)
        self.app.router.add_post("/v1/chat/completions", self.completion)
        self.runner = None

    async def models(self, _request):
        return web.json_response({"object": "list", "data": [{"id": MODEL, "object": "model"}]})

    async def completion(self, request):
        payload = await request.json()
        self.calls.append(payload)
        messages = payload.get("messages", [])
        tool_specs = {item.get("function", {}).get("name"): item.get("function", {})
                      for item in payload.get("tools", [])}
        if self.tool_call and CMDLINE_NAME not in tool_specs:
            raise web.HTTPBadRequest(text=f"expected configured {CMDLINE_NAME} tool")
        # First turn: emit exactly one invocation, then block the actual tool
        # process while the server's foreground generation returns its receipt.
        if self.tool_call and self.emitted == 0:
            self.emitted += 1
            # The cmdline integration's schema consists of the declared command
            # arguments directly; this fixture declares none.
            args = {}
            call = {"id": "call-smoke-once", "type": "function", "function": {
                "name": CMDLINE_NAME, "arguments": json.dumps(args)}}
            response = {"id": "stub-first", "object": "chat.completion", "created": 1,
                "model": MODEL, "choices": [{"index": 0, "finish_reason": "tool_calls",
                    "message": {"role": "assistant", "content": None, "tool_calls": [call]}}]}
        else:
            # User's second turn is served immediately while the first job is
            # blocked; after release the tool result prompts the owning chat again.
            saw_native = any(item.get("role") == "tool" and MARKER in str(item.get("content"))
                             for item in messages)
            saw_second = any(item.get("role") == "user" and SECOND_PROMPT in str(item.get("content", ""))
                             for item in messages)
            if saw_second and not self.barrier.exists():
                self.second_seen_pending = True
            text = ("post-tool-model-continuation" if saw_native else
                    "second-turn-accepted" if saw_second else "first-turn-receipt-acknowledged")
            response = {"id": "stub-followup", "object": "chat.completion", "created": 2,
                "model": MODEL, "choices": [{"index": 0, "finish_reason": "stop",
                    "message": {"role": "assistant", "content": text}}]}
        if payload.get("stream"):
            choice = response["choices"][0]
            delta = dict(choice["message"])
            for index, call in enumerate(delta.get("tool_calls", [])):
                call["index"] = index
            chunk = {**response, "object": "chat.completion.chunk", "choices": [
                {"index": 0, "delta": delta, "finish_reason": None}]}
            end = {**response, "object": "chat.completion.chunk", "choices": [
                {"index": 0, "delta": {}, "finish_reason": choice["finish_reason"]}]}
            return web.Response(content_type="text/event-stream", text=
                f"data: {json.dumps(chunk)}\n\ndata: {json.dumps(end)}\n\ndata: [DONE]\n\n")
        return web.json_response(response)

    async def start(self):
        self.runner = web.AppRunner(self.app)
        await self.runner.setup()
        site = web.TCPSite(self.runner, "127.0.0.1", self.port)
        await site.start()
        self.port = site._server.sockets[0].getsockname()[1]

    async def stop(self):
        if self.runner:
            await self.runner.cleanup()


class Engine:
    def __init__(self, binary: Path, workspace: Path, cache: Path, config: Path, port=None, scheduler=False):
        self.port = port or ephemeral_port()
        self.scheduler = scheduler
        self.binary, self.workspace, self.cache, self.config = binary, workspace, cache, config
        self.process = None

    def start(self):
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", self.port))
        temporary = self.workspace.parent / "tmp"
        temporary.mkdir(exist_ok=True)
        env = {key: os.environ[key] for key in ("PATH", "LANG", "LC_ALL")
               if key in os.environ}
        # Isolate all user config/cache/environment state; configuration written
        # below points only to this process's loopback provider.
        env.update({"TMPDIR": str(temporary), "HOME": str(self.config), "XDG_CONFIG_HOME": str(self.config),
                    "XDG_CACHE_HOME": str(self.cache), "REFACT_CACHE_DIR": str(self.cache),
                    "REFACT_CONFIG_DIR": str(self.config),
                    "HTTP_PROXY": "http://127.0.0.1:9", "HTTPS_PROXY": "http://127.0.0.1:9",
                    "ALL_PROXY": "http://127.0.0.1:9", "NO_PROXY": "127.0.0.1,localhost",
                    "http_proxy": "http://127.0.0.1:9", "https_proxy": "http://127.0.0.1:9",
                    "all_proxy": "http://127.0.0.1:9", "no_proxy": "127.0.0.1,localhost"})
        self.log_path = self.workspace.parent / "engine.log"
        self.log = self.log_path.open("wb")
        self.process = subprocess.Popen([str(self.binary), "--http-port", str(self.port),
            "--workspace-folder", str(self.workspace), * ([] if self.scheduler else ["--no-scheduler"]), "--logs-stderr"],
            cwd=self.workspace, env=env, stdout=self.log, stderr=self.log)
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise AssertionError("owned engine exited before readiness")
            try:
                with socket.create_connection(("127.0.0.1", self.port), timeout=.25):
                    assert owns_listener(self.process.pid, self.port), "port occupied by a different process; refusing adoption"
                    return
            except OSError:
                time.sleep(.1)
        raise AssertionError("owned engine readiness timeout")

    def stop(self):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        assert not self.process or self.process.poll() is not None, "owned worker cleanup unconfirmed"
        if getattr(self, "log", None):
            self.log.close()


class HTTP:
    def __init__(self, session, port=ENGINE_PORT):
        self.session = session
        self.base = f"http://127.0.0.1:{port}/v1"

    async def request(self, method, path, body=None):
        async with self.session.request(method, self.base + path, json=body,
                timeout=aiohttp.ClientTimeout(total=20)) as response:
            text = await response.text()
            data = text if path == "/ping" else json.loads(text)
            assert response.status < 300, f"{method} {path}: HTTP {response.status}: {data}"
            return data

    async def command(self, chat, kind, **kwargs):
        return await self.request("POST", f"/chats/{chat}/commands", {
            "client_request_id": "tool-jobs-smoke-" + uuid.uuid4().hex,
            "type": kind, **kwargs})

    async def snapshot(self, chat):
        async with self.session.get(self.base + "/chats/subscribe", params={"chat_id": chat},
                timeout=aiohttp.ClientTimeout(total=15)) as response:
            assert response.status == 200
            while True:
                line = await response.content.readline()
                assert line, "SSE ended without snapshot"
                if line.startswith(b"data: "):
                    event = json.loads(line[6:])
                    if event.get("type") == "snapshot":
                        return event


async def until(label, callback, seconds=PHASE_TIMEOUT_SECONDS):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = await callback()
        if result:
            return result
        await asyncio.sleep(.1)
    raise AssertionError(label + ": finite timeout")


def write_fixture(workspace: Path, config: Path, cache: Path, provider_port: int):
    workspace.mkdir(parents=True)
    (workspace / ".refact" / "integrations.d").mkdir(parents=True)
    (workspace / ".refact" / "integrations.d" / "cmdline_smoke.yaml").write_text(
        "command: python3 -u barrier_tool.py\ncommand_workdir: "
        + str(workspace) + "\ndescription: Run the deterministic foreground smoke barrier tool\n"
        f"parameters: []\nparameters_required: []\ntimeout: '{COMMAND_TIMEOUT_SECONDS}'\n"
        "available: {on_your_laptop: true, when_isolated: true}\n"
        "confirmation: {ask_user: [], deny: []}\n")
    (workspace / "barrier_tool.py").write_text(
        "import json, pathlib, sys, time\n"
        f"base = pathlib.Path({str(workspace)!r})\n"
        "(base / 'invoked').open('a').write('1\\n')\n"
        "(base / 'started').write_text('ready')\n"
        f"deadline = time.monotonic() + {BARRIER_TIMEOUT_SECONDS}\n"
        "while not (base / 'release').exists() and time.monotonic() < deadline: time.sleep(.025)\n"
        "if not (base / 'release').exists(): raise SystemExit('barrier timed out')\n"
        f"print({MARKER!r}, flush=True)\n")
    provider_dir = config / ".config" / "refact" / "providers.d"
    provider_dir.mkdir(parents=True)
    # Real Rust tokenizers loader, synthetic vocabulary: this tests orchestration,
    # not LLM quality. Never consult or mutate the user's tokenizer cache/network.
    tokenizer = config / "smoke-tokenizer.json"
    tokenizer.write_text(json.dumps({
        "version": "1.0", "truncation": None, "padding": None,
        "added_tokens": [], "normalizer": None,
        "pre_tokenizer": {"type": "Whitespace"}, "post_processor": None,
        "decoder": None,
        "model": {"type": "WordLevel", "vocab": {"[UNK]": 0}, "unk_token": "[UNK]"},
    }))
    (provider_dir / "localstub.yaml").write_text(
        "base_provider: custom\ndisplay_name: Local stub smoke\nenabled: true\napi_key: local-only\n"
        f"chat_endpoint: http://127.0.0.1:{provider_port}/v1/chat/completions\n"
        "wire_format: openai_chat_completions\nenabled_models: [foreground-smoke]\n"
        "custom_models:\n  foreground-smoke:\n    n_ctx: 32768\n    supports_tools: true\n"
        f"    tokenizer: {tokenizer.as_uri()}\n    max_output_tokens: 2048\n")
    (config / ".config" / "refact" / "privacy.yaml").write_text(
        "privacy_rules:\n  zones:\n    - name: local-smoke-only\n      patterns: ['*']\n"
        "      send_to: [localstub]\n      on_shell_read: withhold\n")
    # Project defaults keep the test model explicit and away from any provider
    # discovery/fallback behavior.
    config.mkdir(parents=True, exist_ok=True)
    cache.mkdir(parents=True)




def ephemeral_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def verified_copy(binary, expected, destination):
    assert expected and len(expected) == 64, "explicit binary SHA256 required"
    shutil.copyfile(binary, destination)
    digest = hashlib.sha256(destination.read_bytes()).hexdigest()
    assert digest == expected, "binary differs from explicitly authorized build"
    destination.chmod(0o700)
    return destination


# The required lanes import this fixture explicitly; missing dependencies are errors.
import pytest_asyncio


@pytest_asyncio.fixture
async def owned_engine(request, tmp_path):
    binary = Path(os.environ["REFACT_TEST_BINARY"]).resolve(strict=True)
    expected = os.environ["REFACT_TEST_BINARY_SHA256"]
    copied = verified_copy(binary, expected, tmp_path / "refact-lsp")
    workspace, config, cache = (tmp_path / n for n in ("workspace", "config", "cache"))
    provider = StubProvider(workspace / "release", workspace / "provider-invocations", tool_call=False)
    engine = None
    try:
        await provider.start()
        write_fixture(workspace, config, cache, provider.port)
        engine = Engine(copied, workspace, cache, config,
                        scheduler=request.module.__name__ == "test_scheduler_e2e")
        engine.start()
        engine.base_url = f"http://127.0.0.1:{engine.port}"
        yield engine
    finally:
        primary_error = sys.exception()
        cleanup_errors = []
        try:
            if engine:
                try:
                    engine.stop()
                except BaseException as error:
                    cleanup_errors.append(error)
                logs = Path(__file__).resolve().parents[3] / ".refact/verification/owned-integration"
                logs.mkdir(parents=True, exist_ok=True)
                if getattr(engine, "log_path", None):
                    shutil.copyfile(engine.log_path, logs / (request.node.name + ".log"))
                (logs / (request.node.name + ".identity.json")).write_text(json.dumps({
                    "binary_sha256": expected, "pid": engine.process.pid if engine.process else None,
                    "port": engine.port, "workspace": str(workspace)}))
        except BaseException as error:
            cleanup_errors.append(error)
        finally:
            try:
                await provider.stop()
            except BaseException as error:
                cleanup_errors.append(error)
        if cleanup_errors:
            if primary_error is not None:
                for error in cleanup_errors:
                    primary_error.add_note(f"Owned fixture cleanup failed: {error!r}")
            else:
                raise BaseExceptionGroup("Owned fixture cleanup failed", cleanup_errors)


async def native_messages(client, base_url, chat_id, data):
    messages = data["messages"]
    resolved = []
    for message in messages:
        job = message.get("tool_job")
        if not job:
            resolved.append(message)
            continue
        deadline = time.monotonic() + RECEIPT_TIMEOUT_SECONDS
        while time.monotonic() < deadline:
            response = await client.get(f"{base_url}/v1/chats/{chat_id}/tool-jobs/{job['job_id']}")
            response.raise_for_status()
            state = response.json()
            if state.get("result") is not None:
                assert state["status"] == "succeeded", state
                native = state["result"]["messages"]
                expected_call = message.get("tool_call_id") or job.get("tool_call_id")
                assert expected_call, f"receipt lacks tool call identity: {message}"
                assert native and all(m.get("tool_call_id") == expected_call for m in native), state
                resolved.extend(native)
                break
            await asyncio.sleep(.1)
        else:
            raise AssertionError(f"native tool job did not complete: {job}")
    assert resolved, data
    assert all(not m.get("tool_failed") for m in resolved), resolved
    return resolved


def owns_listener(pid, port):
    # Linux /proc is authoritative: readiness must belong to this spawned PID.
    proc = Path("/proc")
    assert proc.is_dir(), "owned listener identity requires Linux /proc"
    inodes = set()
    for line in (proc / "net/tcp").read_text().splitlines()[1:]:
        fields = line.split()
        if fields[1] == f"0100007F:{port:04X}" and fields[3] == "0A":
            inodes.add(fields[9])
    try:
        descriptors = list((proc / str(pid) / "fd").iterdir())
    except (PermissionError, FileNotFoundError):
        return False
    for fd in descriptors:
        try:
            if os.readlink(fd) in {f"socket:[{inode}]" for inode in inodes}:
                return True
        except FileNotFoundError:
            pass
    return False


class RequiredLaneResults:
    def __init__(self):
        self.skipped = []
        self.passed = 0

    def pytest_runtest_logreport(self, report):
        if report.skipped:
            self.skipped.append(report.nodeid)
        if report.when == "call" and report.passed:
            self.passed += 1

    def pytest_collectreport(self, report):
        if report.skipped:
            self.skipped.append(report.nodeid)


def main():
    import pytest
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--binary-sha256", required=True)
    args = parser.parse_args()
    tests = Path(__file__).resolve().parent
    logs = tests.parents[2] / ".refact/verification/owned-integration"
    logs.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="owned-integration-run-") as directory:
        binary = verified_copy(args.binary.resolve(strict=True), args.binary_sha256,
                               Path(directory) / "refact-lsp")
        os.environ["REFACT_TEST_BINARY"] = str(binary)
        os.environ["REFACT_TEST_BINARY_SHA256"] = args.binary_sha256
        identity = subprocess.run([str(binary), "--version"], check=True,
                                  capture_output=True, text=True).stdout.strip()
        results = RequiredLaneResults()
        code = pytest.main(["-q", "--tb=short", "--junitxml=" + str(logs / "results.xml"),
                            *[str(tests / name) for name in (
                                "test_owned_engine.py", "test_tool_jobs_smoke.py",
                                "test_background_notifications_e2e.py", "test_pty_stdin_e2e.py",
                                "test_scheduler_e2e.py", "test_hidden_roles_e2e.py")]], plugins=[results])
        summary = {"binary_sha256": args.binary_sha256, "version": identity,
                   "passed": results.passed, "skipped": results.skipped, "pytest_exit": int(code)}
        (logs / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(json.dumps(summary), flush=True)
        raise SystemExit(int(code) or int(bool(results.skipped)) or int(results.passed != 18))


if __name__ == "__main__":
    main()

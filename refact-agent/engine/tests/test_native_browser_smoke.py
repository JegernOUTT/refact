import argparse
import base64
import json
from pathlib import Path
import tempfile
import threading
import traceback
import unittest
import urllib.error
import urllib.parse
import urllib.request
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


UNICODE = "smoke 雪 café 🐸"
LARGE = "雪🐸café-" * 5000
HTML = r"""<!doctype html><meta charset=utf-8><title>Native smoke</title>
<label>Name<input id=name></label><button id=inc onclick="++window.count">Increment</button>
<iframe id=frame src=/frame></iframe><div id=host></div>
<script>window.count=0;window.marker='';
const root=document.querySelector('#host').attachShadow({mode:'open'});
const button=document.createElement('button');button.id='shadow';button.textContent='Shadow';
button.addEventListener('click',()=>button.textContent='shadow done');root.append(button);
</script>"""


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def text_report(response, call_id):
    require(isinstance(response.get("messages"), list), "missing tool messages")
    messages = [m for m in response["messages"]
                if m.get("role") == "tool" and m.get("tool_call_id") == call_id]
    require(len(messages) == 1, "expected exactly one matching tool response")
    content = messages[0]["content"]
    if isinstance(content, list):
        texts = [e.get("text", e.get("m_content")) for e in content
                 if e.get("type", e.get("m_type")) == "text"]
        require(len(texts) == 1 and isinstance(texts[0], str), "one JSON text required")
        content = texts[0]
    require(isinstance(content, str), "tool content is not text")
    report = json.loads(content)
    require(isinstance(report, dict) and report.get("tool") == "chrome", "not chrome JSON")
    return report, len(content.encode())


def check_completion(report, count, failed=0, skipped=0):
    c = report["completion"]
    require(c["steps_requested"] == count and c["steps_reported"] == count,
            "missing executed/reported steps")
    require(c["failed"] == failed and c["skipped"] == skipped and c["unknown"] == 0,
            f"unexpected completion: {c}")
    require(c["incomplete"] is False, "incomplete batch")
    require(len(report["steps"]) == count, "step details omitted; inspect raw receipt")
    require(report["ok"] is (failed == 0 and skipped == 0), "incorrect batch verdict")
    if not failed and not skipped:
        require(all(s["ok"] and not s.get("skipped", False) for s in report["steps"]),
                "hidden non-success step")


class Fixture(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        path = urllib.parse.urlsplit(self.path).path
        body, mime = {
            "/": (HTML, "text/html"),
            "/frame": ('<meta charset=utf-8><label>Frame<input id=inside></label>', "text/html"),
            "/api": (json.dumps({"value": UNICODE}, ensure_ascii=False), "application/json"),
            "/large": (LARGE, "text/plain"),
            "/cookie": ('{"synthetic":true}', "application/json"),
        }.get(path, ("not found", "text/plain"))
        data = body.encode()
        self.send_response(200 if path in ("/", "/frame", "/api", "/large", "/cookie") else 404)
        self.send_header("Content-Type", mime + "; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        if path == "/cookie":
            self.send_header("Set-Cookie", "native_smoke=synthetic; Path=/; SameSite=Lax")
        self.end_headers()
        self.wfile.write(data)


class Smoke:
    def __init__(self, args):
        self.args = args
        self.chat = "native-smoke-" + uuid.uuid4().hex
        self.out = Path(args.output_dir or tempfile.mkdtemp(prefix=self.chat + "-"))
        self.out.mkdir(parents=True, exist_ok=True)
        self.seq, self.generation, self.owned = 0, None, set()
        self.metrics = {"chat_id": self.chat, "calls": [], "generation_changes": [],
                        "expected_failures": 0, "executed_steps": 0, "cleanup_errors": []}
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def http_json(self, path, payload=None):
        self.seq += 1
        stem = self.out / f"{self.seq:03d}"
        data = None if payload is None else json.dumps(payload, ensure_ascii=False).encode()
        if data is not None:
            stem.with_suffix(".request.json").write_bytes(data)
        request = urllib.request.Request(self.args.url.rstrip("/") + path, data=data,
                                         headers={"Content-Type": "application/json"})
        try:
            with self.http.open(request, timeout=90) as response:
                raw = response.read(12 * 1024 * 1024 + 1)
        except urllib.error.HTTPError as error:
            stem.with_suffix(".response.raw").write_bytes(error.read())
            raise
        stem.with_suffix(".response.raw").write_bytes(raw)
        require(len(raw) <= 12 * 1024 * 1024, "HTTP byte budget exceeded")
        return json.loads(raw)

    def call(self, steps, failed=0, skipped=0, **options):
        require(0 < len(steps) <= 64, "invalid batch size")
        request = {"steps": steps, "page_context": "none", "network": "none",
                   "attach_screenshot": False, **options}
        call_id = "call_" + uuid.uuid4().hex
        payload = {"messages": [{"role": "assistant", "content": "", "tool_calls": [
            {"id": call_id, "type": "function", "function": {"name": "chrome",
             "arguments": json.dumps({"request": request}, ensure_ascii=False)}}]}],
            "n_ctx": 131072, "maxgen": 4096, "subchat_tool_parameters": {},
            "postprocess_parameters": {}, "model_name": self.args.model, "chat_id": self.chat}
        report, size = text_report(self.http_json("/v1/tools-execute", payload), call_id)
        self.metrics["calls"].append({"steps": len(steps), "report_bytes": size,
                                       "completion": report.get("completion")})
        for step in report.get("steps", []):
            index = step.get("step_index")
            if isinstance(index, int) and index < len(steps) and steps[index]["action"] == "open_tab":
                tab = (step.get("data") or {}).get("tab_id")
                if tab:
                    self.owned.add(tab)
        generation = report.get("session_generation")
        require(isinstance(generation, str) and generation, "missing session generation")
        if self.generation is not None and generation != self.generation:
            self.metrics["generation_changes"].append([self.generation, generation])
            raise AssertionError("unexpected runtime replacement; no automatic replay")
        self.generation = generation
        check_completion(report, len(steps), failed, skipped)
        self.metrics["executed_steps"] += len(steps) - skipped
        self.metrics["expected_failures"] += failed
        return report

    def oracle(self, expression):
        return {"action": "eval", "expression":
                f"(async () => {{if (!(await ({expression}))) throw Error('smoke oracle'); return true}})()"}

    def open_owned(self, url):
        report = self.call([{"action": "open_tab", "url": url}])
        tab = report["steps"][0]["data"]["tab_id"]
        self.owned.add(tab)
        return tab

    def close_owned(self, tab):
        require(tab in self.owned, "refuse unowned tab cleanup")
        self.call([{"action": "close_tab", "tab": {"type": "id", "id": tab}}])
        self.owned.remove(tab)

    def retrieve(self, artifact):
        total, offset, chunks = artifact["bytes"], 0, []
        require(0 < total <= 512 * 1024, "full report byte budget")
        for _ in range(80):
            report = self.call([{"action": "read_artifact", "id": artifact["id"],
                                 "offset": offset, "limit": 8191}])
            require("full_report" not in report, "recursive retrieval artifact")
            data = report["steps"][0]["data"]
            chunk = base64.b64decode(data["content_base64"], validate=True)
            require(data["total_bytes"] == total and chunk, "invalid artifact bytes")
            chunks.append(chunk)
            offset += len(chunk)
            require(offset <= total, "artifact overflow")
            if data.get("next_offset") is None:
                require(offset == total, "artifact truncated")
                raw = b"".join(chunks)
                (self.out / "full-report.json").write_bytes(raw)
                return json.loads(raw.decode("utf-8"))
            require(data["next_offset"] == offset, "non-byte cursor")
        raise AssertionError("artifact chunk budget exhausted")

    def run(self, base):
        tools = self.http_json("/v1/tools")
        chrome = [t for g in tools for t in g["tools"] if t["spec"]["name"] == "chrome"]
        require(len(chrome) == 1 and chrome[0]["enabled"], "live chrome unavailable")
        schema = json.dumps(chrome[0]["spec"])
        require("read_artifact" in schema and "acknowledge_session_generation" in schema,
                "stale native tool schema")
        main = self.open_owned(base)
        self.call([{"action": "fill", "locator": {"by": "id", "value": "name"}, "text": UNICODE},
                   {"action": "expect", "locator": {"by": "id", "value": "name"},
                    "matcher": {"type": "to_have_value", "expected": UNICODE}},
                   {"action": "fill", "locator": {"by": "id", "value": "inside", "frames": [
                       {"by": "id", "value": "frame"}]}, "text": UNICODE},
                   {"action": "click", "locator": {"by": "id", "value": "shadow"}},
                   {"action": "eval", "expression": "window.marker=" + json.dumps(self.chat)},
                   self.oracle("document.querySelector('#host').shadowRoot.querySelector('button').textContent==='shadow done'")])
        self.call([{"action": "set_viewport", "width": 900, "height": 700},
                   self.oracle("window.marker===" + json.dumps(self.chat)),
                   self.oracle("document.querySelector('#name').value===" + json.dumps(UNICODE)),
                   self.oracle("document.querySelector('#frame').contentDocument.querySelector('input').value===" + json.dumps(UNICODE)),
                   self.oracle("fetch('/api').then(r=>r.json()).then(x=>{if(x.value!==" + json.dumps(UNICODE) + ")throw Error('api');return true})")])
        self.call([self.oracle("fetch('/cookie').then(r=>r.json()).then(()=>{if(!document.cookie.includes('native_smoke=synthetic'))throw Error('cookie');return true})")])
        failure = {"action": "expect", "locator": {"by": "id", "value": "name"},
                   "matcher": {"type": "to_have_value", "expected": "deliberately wrong"}, "timeout_ms": 100}
        self.call([failure, {"action": "eval", "expression": "window.mustNotRun=true"}], failed=1, skipped=1)
        self.call([self.oracle("window.mustNotRun===undefined")])
        self.call([failure, {"action": "eval", "expression": "window.continued=true"}],
                  failed=1, continue_on_error=True)
        self.call([self.oracle("window.continued===true")])
        for index in range(self.args.rounds):
            steps = []
            for _ in range(24):
                steps.extend([{"action": "click", "locator": {"by": "id", "value": "inc"}},
                              self.oracle("window.marker===" + json.dumps(self.chat))])
            steps.append({"action": "expect_poll", "expression": "window.count",
                          "matcher": "equals", "expected": (index + 1) * 24,
                          "timeout_ms": 1000})
            self.call(steps)
            churn = self.open_owned(base + "frame")
            self.close_owned(churn)
            self.call([{"action": "switch_tab", "tab": {"type": "id", "id": main}},
                       self.oracle("window.marker===" + json.dumps(self.chat))])
        report = self.call([{"action": "eval", "expression": "fetch('/large').then(r=>r.text())"}])
        require("full_report" in report, "large report did not create retained artifact")
        full = self.retrieve(report["full_report"])
        check_completion(full, 1)
        require(full["steps"][0]["data"]["value"] == LARGE, "UTF-8 report content changed")
        shot = self.call([{"action": "screenshot"}])
        image = shot["steps"][0]["data"]["artifact"]
        require(image.get("mime", "").startswith("image/") and image.get("id") and image.get("path"),
                "missing screenshot artifact metadata")
        require(Path(image["path"]).is_file(), "screenshot artifact does not exist")
        self.close_owned(main)
        tabs = self.call([{"action": "list_tabs"}])["steps"][0]["data"]["tabs"]
        self.metrics["missing_active_exercised"] = not tabs
        if not tabs:
            self.call([{"action": "eval", "expression": "1"}], failed=1)
        replacement = self.open_owned("about:blank")
        self.call([self.oracle("location.href==='about:blank'")])
        self.close_owned(replacement)


def self_check():
    class ParserTests(unittest.TestCase):
        def test_text_shapes(self):
            text = json.dumps({"tool": "chrome", "ok": True})
            for content in (text, [{"type": "text", "text": text}],
                            [{"m_type": "text", "m_content": text},
                             {"m_type": "image/png", "m_content": "not JSON"}]):
                report, _ = text_report({"tools_ran": False, "messages": [
                    {"role": "tool", "tool_call_id": "x", "content": content}]}, "x")
                self.assertTrue(report["ok"])
            with self.assertRaises((AssertionError, json.JSONDecodeError)):
                text_report({"tools_ran": True, "messages": [{"role": "tool",
                    "tool_call_id": "x", "content": text + " trailing"}]}, "x")

        def test_fixture_and_async_oracle(self):
            require("await (Promise.resolve(false))" in Smoke.oracle(None, "Promise.resolve(false)")["expression"],
                    "oracle must await rejected/false promises")
            server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{server.server_port}/large", timeout=2) as response:
                    self.assertEqual(response.read(), LARGE.encode())
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=2)

        def test_byte_exact_retrieval(self):
            raw = json.dumps({"value": UNICODE}, ensure_ascii=False).encode()
            with tempfile.TemporaryDirectory() as directory:
                fake = type("Fake", (), {"out": Path(directory)})()
                def chunk(steps):
                    offset = steps[0]["offset"]
                    data = raw[offset:offset + 7]
                    return {"steps": [{"data": {"content_base64": base64.b64encode(data).decode(),
                        "total_bytes": len(raw), "next_offset": offset + len(data)
                        if offset + len(data) < len(raw) else None}}]}
                fake.call = chunk
                self.assertEqual(Smoke.retrieve(fake, {"id": "synthetic", "bytes": len(raw)}),
                                 {"value": UNICODE})
                self.assertEqual((fake.out / "full-report.json").read_bytes(), raw)

        def test_no_skipped_success(self):
            report = {"ok": True, "steps": [], "completion": {"steps_requested": 1,
                "steps_reported": 1, "failed": 0, "skipped": 1, "unknown": 0, "incomplete": False}}
            with self.assertRaises(AssertionError):
                check_completion(report, 1)
    result = unittest.TextTestRunner().run(unittest.defaultTestLoader.loadTestsFromTestCase(ParserTests))
    return 0 if result.wasSuccessful() else 1


def main():
    parser = argparse.ArgumentParser(description="Finite native chrome smoke; use an isolated worker only")
    parser.add_argument("--url")
    parser.add_argument("--model", default="")
    parser.add_argument("--rounds", type=int, default=4)
    parser.add_argument("--output-dir")
    parser.add_argument("--self-check", action="store_true")
    args = parser.parse_args()
    if args.self_check:
        return self_check()
    parsed = urllib.parse.urlsplit(args.url or "")
    require(parsed.scheme == "http" and parsed.hostname in ("127.0.0.1", "localhost", "::1")
            and parsed.port not in (None, 8001), "provide explicit isolated loopback worker URL (not :8001)")
    require(1 <= args.rounds <= 8, "rounds must be 1..8")
    runner, error = Smoke(args), None
    server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        runner.run(f"http://127.0.0.1:{server.server_port}/")
    except Exception:
        error = traceback.format_exc()
    finally:
        for tab in list(runner.owned):
            try:
                runner.close_owned(tab)
            except Exception as cleanup:
                runner.metrics["cleanup_errors"].append(str(cleanup))
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)
        runner.metrics.update({"passed": error is None and not runner.metrics["cleanup_errors"], "error": error})
        (runner.out / "summary.json").write_text(json.dumps(runner.metrics, indent=2), encoding="utf-8")
    print(json.dumps({"passed": runner.metrics["passed"], "output_dir": str(runner.out), "error": error}))
    return 0 if runner.metrics["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

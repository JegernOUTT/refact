#!/usr/bin/env python3
"""Opt-in real-model integration runner: python tests/test_review_tool_smoke.py --help.

Requires a dedicated updated server whose sole workspace is --smoke-root, and
an already configured real --model. Never run against a user's active chat.
Scope answers are SIMULATED fixture consent through the public submit_question_response
command, not real user approval or evidence of product/user success. No provider
configuration or credentials are read. Fixtures and redacted call logs are kept
for diagnosis; this is intentionally not a default quick-CI test.
"""
import argparse
import asyncio
import json
from pathlib import Path
import re
import shlex
import subprocess
import time
import uuid
from urllib.parse import urlparse

import aiohttp

TOOLS = {"set_review_context", "review_get", "review_anchor", "review_update",
         "review_reply", "review_prepare_execution", "review_record_evidence"}
ACTIONS = {"stack_prepare", "stack_import", "stack_inspect", "stack_member",
           "stack_capture", "stack_cumulative", "stack_refresh"}
OPS = {"rename", "set_target_reference", "put_tour_step", "put_annotation", "put_thread", "add_evidence"}
HISTORICAL_REFUSALS = {
    "stack_cumulative": "Cumulative projection is unavailable for recorded historical comparisons",
    "stack_refresh": "Recorded historical comparisons are immutable",
}


def source_identity(root):
    return (git(root, "rev-parse", "HEAD"), git(root, "write-tree"),
            (root / "calculator.py").read_bytes(),
            (root / ".git" / "config").read_bytes())


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], stderr=subprocess.STDOUT,
                                   timeout=10, text=True).strip()


def fixture(root):
    root.mkdir()
    git(root, "init", "-q")
    git(root, "config", "user.name", "Review Smoke")
    git(root, "config", "user.email", "review-smoke@localhost")
    path = root / "calculator.py"
    path.write_text("def add(a, b):\n    return a + b\n\nassert add(2, 3) == 5\n")
    git(root, "add", ".")
    git(root, "commit", "-qm", "base")
    base = git(root, "rev-parse", "HEAD")
    path.write_text("def add(a, b):\n    return a - b\n\nassert add(2, 3) == 5\n")
    git(root, "commit", "-qam", "member one regression")
    one = git(root, "rev-parse", "HEAD")
    git(root, "branch", "smoke-one", one)
    (root / "note.txt").write_text("second stack member\n")
    git(root, "add", ".")
    git(root, "commit", "-qm", "member two")
    two = git(root, "rev-parse", "HEAD")
    git(root, "branch", "smoke-two", two)
    tree = git(root, "rev-parse", base + "^{tree}")
    divergent = git(root, "commit-tree", tree, "-p", base, "-m", "divergent recorded base")
    assert divergent != one
    return {"repository": str(root), "base": base, "one": one, "two": two,
            "divergent": divergent}


def text(message):
    content = message.get("content", "")
    return content if isinstance(content, str) else json.dumps(content)


def calls(snapshot):
    messages = snapshot.get("messages", [])
    results = {m.get("tool_call_id"): m for m in messages if m.get("role") == "tool"}
    found = []
    for message in messages:
        for call in message.get("tool_calls") or []:
            function = call.get("function", {})
            args = function.get("arguments", {})
            try:
                args = json.loads(args) if isinstance(args, str) else args
            except ValueError:
                args = {"invalid_arguments": True}
            result = results.get(call["id"])
            value = None
            if result:
                try:
                    value = json.loads(text(result))
                except ValueError:
                    pass
            successful = (result is not None and value is not None
                          and not result.get("tool_failed")
                          and not result.get("tool_call_error")
                          and not result.get("is_error")
                          and not (isinstance(value, dict) and (value.get("error") or value.get("detail"))))
            found.append({"id": call["id"], "name": function.get("name", "").removeprefix("t_"), "args": args,
                          "success": successful, "value": value,
                          "result_text": text(result or {})[:12000],
                          "error": None if successful else text(result or {})[:1500]})
    return found


class Runner:
    def __init__(self, options, session):
        self.o, self.http = options, session
        self.chat = None
        self.deadline = 0
        self.turns = 0
        self.answered = set()
        self.answer_receipts = []

    async def request(self, method, path, body=None):
        async with self.http.request(method, self.o.url + "/v1" + path, json=body,
                                     timeout=aiohttp.ClientTimeout(total=15)) as response:
            data = await response.json()
            if response.status >= 300:
                raise AssertionError(f"{method} {path}: HTTP {response.status}: {str(data)[:1000]}")
            return data

    async def command(self, kind, **kwargs):
        return await self.request("POST", f"/chats/{self.chat}/commands", {
            "client_request_id": "review-smoke-" + str(uuid.uuid4()), "type": kind, **kwargs})

    async def snapshot(self):
        async with self.http.get(self.o.url + "/v1/chats/subscribe", params={"chat_id": self.chat},
                                 timeout=aiohttp.ClientTimeout(total=15)) as response:
            assert response.status == 200
            while True:
                line = await response.content.readline()
                if not line:
                    raise AssertionError("SSE ended without snapshot")
                if line.startswith(b"data: "):
                    event = json.loads(line[6:])
                    if event.get("type") == "snapshot":
                        return event

    async def wait(self, scope):
        started = time.monotonic()
        while time.monotonic() < self.deadline:
            snap = await self.snapshot()
            self.save(snap)
            assert len(calls(snap)) <= 100, "Actual tool-call budget exceeded"
            for question_set in snap.get("pending_questions", []):
                if question_set.get("status") != "pending" or question_set["tool_call_id"] in self.answered:
                    continue
                answers = []
                for question in question_set["questions"]:
                    options = question.get("options", [])
                    wanted = "Review the whole stack" if scope == "whole" else "Review this PR only"
                    choices = [v for v in options if v == wanted]
                    assert len(choices) == 1, f"Cannot safely simulate scope answer: {question}"
                    choice = choices[0]
                    answers.append({"id": question["id"], "disposition": "answered",
                                    "answer": choice.get("id", choice.get("value")) if isinstance(choice, dict) else choice})
                receipt = await self.command("submit_question_response", tool_call_id=question_set["tool_call_id"],
                                             asked_at_ms=question_set["asked_at_ms"],
                                             response={"answers": answers, "overall_comments": "SIMULATED fixture scope consent; not real user approval."})
                self.answered.add(question_set["tool_call_id"])
                self.answer_receipts.append({"fixture_simulated_consent": True,
                                             "tool_call_id": question_set["tool_call_id"],
                                             "answers": answers, "public_command_receipt": receipt})
                self.save(snap)
                break
            else:
                state = snap.get("runtime", {}).get("state")
                if state in ("idle", "completed", "paused", "error") and time.monotonic() - started > 2:
                    assert state in ("idle", "completed"), f"Chat stopped in {state}: {snap.get('runtime')}"
                    return snap
            await asyncio.sleep(1)
        raise AssertionError(f"Scenario deadline exceeded; see {self.o.logs / (self.chat + '.json')}")

    def save(self, snap):
        safe = {"chat_id": self.chat, "fixture_simulated_consent": True,
                "model": self.o.model, "simulated_answer_receipts": self.answer_receipts,
                "runtime": snap.get("runtime"), "calls": calls(snap)}
        (self.o.logs / (self.chat + ".json")).write_text(json.dumps(safe, indent=2))

    async def turn(self, prompt, scope="single"):
        self.turns += 1
        assert self.turns <= self.o.max_turns, "Model turn budget exceeded"
        await self.command("user_message", content=prompt)
        return await self.wait(scope)

    async def scenario(self, name, f, specs):
        source_root = Path(f["repository"])
        self.before_source = source_identity(source_root)
        self.answered = set()
        self.answer_receipts = []
        self.chat = "review-smoke-" + name + "-" + uuid.uuid4().hex
        self.deadline = time.monotonic() + self.o.seconds
        self.turns = 0
        await self.snapshot()
        await self.command("set_params", patch={"model": self.o.model, "mode": "review",
                                               "auto_approve_editing_tools": True,
                                               "auto_approve_dangerous_commands": True})
        contract = json.dumps(specs)
        safety = ("Disposable smoke fixture. Do not publish, fetch, push, download, mutate remotes, or checkout the source repository. "
                  "Use only advertised tools and their schemas below. Actually call tools; prose is not execution. "
                  "Scope answers will be simulated by this fixture client, not real user approval. "
                  "Use isolated registered investigation checkout for experiments. " + contract)
        if name == "local":
            task = (f"Review local repository {f['repository']} base {f['base']} candidate {f['one']}. "
                    "Activate normal comparison, read summary and calculator.py immutable range, create validated anchor. "
                    "Use EVERY allowed review_update operation: rename, set_target_reference, put_tour_step, put_annotation finding, put_thread, add_evidence source. "
                    "Read exact thread and reply using review_reply. Prepare execution, run python3 calculator.py in investigation checkout and retain failed investigation evidence. "
                    "Repair subtraction to addition only in investigation checkout, prepare again, run and record successful repair evidence. "
                    "Read immutable source again to establish it remains subtraction; don't claim original passes. Finish after completing all.")
        else:
            historical = name == "historical"
            members = [{"key": "one", "branch_ref": "smoke-one", "parent": None},
                       {"key": "two", "branch_ref": "missing-peer" if name == "single" else "smoke-two", "parent": "one"}]
            discovery = {"title": "Fixture stack", "source": {"kind": "provided_manifest", "reference": "disposable fixture"},
                         "selected_member": "one", "members": members,
                         "comparison_mode": "recorded_comparisons" if historical else "live_chain"}
            comparisons = [{"member": "one", "base_ref": f["base"], "head_ref": f["one"]}]
            if name != "single":
                comparisons.append({"member": "two", "base_ref": f["divergent"] if historical else f["one"], "head_ref": f["two"]})
            task = (f"Known local stack at {f['repository']}. Discovery source: {json.dumps(discovery)}. "
                    "FIRST stack_prepare using public discovery DTO, then ask_questions with exact returned question and stop for fixture answer BEFORE acquisition/capture. "
                    f"After answered receipt import using pinned comparisons {json.dumps(comparisons)} and root base {f['base']}. "
                    "Inspect stack and member, capture selected member and read review_get. "
                    + ("Nonselected missing-peer is deliberately inaccessible: do not resolve, acquire or capture it." if name == "single" else
                       "Capture both members, read cumulative comparison for second member. Refresh explicit manifest with current revision and stable key, retry exact same refresh key/payload once to prove replay, inspect again. ")
                    + ("These are historical merged recorded comparisons: second base is divergent from first head; preserve source-disclosed bases, never silently substitute live-chain parent." if historical else ""))
        if name == "local":
            snap = await self.turn(safety + f"FIRST use existing shell tool to execute git -C {shlex.quote(f['repository'])} remote -v (local config listing ONLY, no network). Then activate ONLY normal review comparison repository {f['repository']} base {f['base']} candidate {f['one']}, then read calculator.py and return a validated review_anchor. Stop; fixture client will seed a simulated user comment.")
            remote_results = [c["result_text"] for c in calls(snap)
                              if c["name"] == "shell" and "remote -v" in c["args"].get("command", "")]
            assert remote_results, "No actual shell remote-listing result"
            for output in remote_results:
                assert "invalid.test/repo" in output, "Redaction erased host/path or remote listing failed"
                assert "FAKE_SMOKE_CREDENTIAL" not in output, "Synthetic credential leaked in actual tool result"
                assert not re.search(r"https?://[^/\s]*@", output), "URI userinfo leaked in actual tool result"
            activated = [c for c in calls(snap) if c["success"] and c["name"] == "set_review_context"]
            anchors = [c for c in calls(snap) if c["success"] and c["name"] == "review_anchor"]
            assert activated and anchors, "Model did not activate and anchor before fixture comment"
            review = await self.request("GET", f"/reviews/by-chat/{self.chat}")
            await self.request("POST", f"/reviews/{review['id']}/comments", {
                "expected_revision": review["revision"], "request_key": "simulated-fixture-comment",
                "anchor": anchors[-1]["value"], "body": "SIMULATED fixture user question: explain this regression; not real user consent."})
            task = task.replace("put_thread,", "put_thread updating the EXISTING simulated user thread (never create one),")
            snap = await self.turn(task)
        else:
            if name == "historical":
                task = task.replace("read cumulative comparison for second member. Refresh explicit manifest with current revision and stable key, retry exact same refresh key/payload once to prove replay, inspect again.",
                                    "attempt cumulative and refresh once each and expect explicit historical refusal; preserve pinned recorded comparisons and inspect again.")
            snap = await self.turn(safety + task, "single" if name == "single" else "whole")
        required = TOOLS if name == "local" else {"set_review_context", "review_get"}
        for _ in range(self.o.max_turns - self.turns + 1):
            successful = [c for c in calls(snap) if c["success"]]
            missing = required - {c["name"] for c in successful}
            if name != "local":
                missing |= (ACTIONS - {c["args"].get("action") for c in successful}) - ({"stack_refresh", "stack_cumulative"} if name in ("single", "historical") else set())
            else:
                missing |= OPS - {c["args"].get("operation", {}).get("op") for c in successful if c["name"] == "review_update"}
                durable = await self.request("GET", f"/reviews/by-chat/{self.chat}")
                durable = durable.get("review", durable)
                materials = {m["id"]: m for m in durable.get("materials", [])}
                for kind, code in (("investigation", 1), ("repair", 0)):
                    if not any(e.get("source", {}).get("kind") == "executed"
                               and e["source"].get("exit_code") == code
                               and materials.get(e.get("material_id"), {}).get("kind") == kind
                               for e in durable.get("evidence", [])):
                        missing.add(f"record_{kind}_exit_{code}_evidence")
                recorded = [i for i, c in enumerate(calls(snap))
                            if c["success"] and c["name"] == "review_record_evidence"]
                if not recorded or not any(c["success"] and c["name"] == "review_get"
                                           and c["args"].get("path") == "calculator.py"
                                           and c["value"].get("side") == "head"
                                           for c in calls(snap)[recorded[-1] + 1:]):
                    missing.add("reread_immutable_head_after_evidence")
            if not missing:
                break
            snap = await self.turn("Continue actual smoke tool execution; missing successful calls: " + ", ".join(sorted(missing)) + ". Repair errors using advertised schema. " + task,
                                   "single" if name in ("local", "single") else "whole")
        assert not missing, f"Missing successful calls: {missing}; see {self.chat}.json"
        actual = calls(snap)
        if name != "local":
            first_capture = next(i for i, c in enumerate(actual) if c["args"].get("action") == "stack_capture")
            import_index = next(i for i, c in enumerate(actual) if c["success"] and c["args"].get("action") == "stack_import")
            assert import_index < first_capture, "Capture attempted before scope import"
            imported = actual[import_index]
            receipt = imported["args"].get("scope_receipt_tool_call_id") or imported["args"].get("acquisition", {}).get("scope_receipt_tool_call_id")
            assert receipt in self.answered, "Import lacks actual simulated answered endpoint receipt"
            if name == "historical":
                for action in ("stack_cumulative", "stack_refresh"):
                    refused = [c for c in actual if c["name"] == "set_review_context"
                               and c["args"].get("action") == action and not c["success"]
                               and HISTORICAL_REFUSALS[action] in (c["error"] or "")]
                    assert refused, f"Missing explicit historical refusal for {action}"
            if name == "whole":
                captures = [c for c in actual if c["success"] and c["args"].get("action") == "stack_capture"]
                assert len({c["value"]["candidate_id"] for c in captures}) == 2, "Both member comparisons not captured"
                refreshes = [c for c in actual if c["success"] and c["args"].get("action") == "stack_refresh"]
                assert len(refreshes) >= 2, "Missing actual refresh replay"
                assert refreshes[-1]["args"] == refreshes[-2]["args"], "Replay payload changed"
                assert refreshes[-1]["value"] == refreshes[-2]["value"], "Replay changed durable manifest"
        review = await self.request("GET", f"/reviews/by-chat/{self.chat}")
        (self.o.logs / (self.chat + "-durable.json")).write_text(json.dumps(review, indent=2))
        review = review.get("review", review)
        if name == "local":
            assert review.get("tour") and review.get("annotations") and review.get("threads")
            assert any(t.get("contributions") for t in review["threads"])
            executed = [e for e in review.get("evidence", []) if e.get("source", {}).get("kind") == "executed"]
            candidate = review["current_candidate_id"]
            materials = {m["id"]: m for m in review["materials"]}
            bound = []
            for evidence in executed:
                source = evidence["source"]
                command = source.get("command", [])
                argv = shlex.split(command[0]) if len(command) == 1 else command
                if argv != ["python3", "calculator.py"]:
                    continue
                material = materials.get(evidence.get("material_id"), {})
                assert evidence["candidate_id"] == material.get("candidate_id") == candidate
                prepared_id = material.get("input_digests", {}).get("prepared_material_id")
                assert prepared_id in materials, "Missing prepared input binding"
                assert materials[prepared_id]["candidate_id"] == candidate
                assert source["workdir"] == material.get("worktree_root")
                assert Path(source["workdir"]).resolve() != source_root.resolve(), "Execution used original source"
                bound.append((evidence, material))
            failing = [(e, m) for e, m in bound if e["source"].get("exit_code") == 1
                       and m["kind"] == "investigation"]
            passing = [(e, m) for e, m in bound if e["source"].get("exit_code") == 0
                       and m["kind"] == "repair"]
            assert failing, "No actual python3 calculator.py exit-1 investigation evidence"
            assert passing, "No actual python3 calculator.py exit-0 repair evidence"
            assert failing[-1][1]["content_digest"] != passing[-1][1]["content_digest"], "Repair inputs did not change"
            reads = [c for c in actual if c["success"] and c["name"] == "review_get"
                     and c["args"].get("path") == "calculator.py"
                     and c["value"].get("candidate_id") == candidate
                     and c["value"].get("side") == "head"]
            assert reads and any(line["text"].strip() == "return a - b"
                                 for line in reads[-1]["value"].get("lines", [])), "Immutable candidate subtraction not verified"
            last_record = max(i for i, c in enumerate(actual)
                              if c["success"] and c["name"] == "review_record_evidence")
            assert actual.index(reads[-1]) > last_record, "Immutable source was not reread after experiment evidence"
            assert all(e.get("applicability") != "current" for e in executed)
            assert any(m.get("kind") == "repair" for m in review["materials"])
        else:
            stack = await self.request("GET", f"/review-stacks/by-chat/{self.chat}")
            (self.o.logs / (self.chat + "-stack.json")).write_text(json.dumps(stack, indent=2))
            stack = stack.get("stack", stack)
            assert stack["scope_choice"]["confirmed"]
            if name == "single":
                assert stack["scope_choice"]["scope"] == "single"
                assert sum(bool(m.get("review_id")) for m in stack["members"]) == 1
            if name == "historical":
                second = next(m for m in stack["members"] if m["branch_ref"] == "smoke-two")
                assert second["resolved_base_oid"] == f["divergent"], "Historical base silently rewritten"
        assert source_identity(source_root) == self.before_source, "Source HEAD/tree/calculator bytes mutated"
        assert git(source_root, "status", "--porcelain", "--untracked-files=no") == "", "Source tracked files mutated"
        print(f"PASS {name}: {self.chat} (fixture/simulated consent, actual model calls and durable results)")


async def main(options):
    parsed = urlparse(options.url)
    assert parsed.hostname in ("localhost", "127.0.0.1", "::1"), "Dedicated loopback server required"
    assert parsed.port not in (8005, 5205), "Do not use existing user servers"
    root = options.smoke_root.resolve(strict=True)
    assert (root / ".review-smoke-approved").is_file(), "Create .review-smoke-approved in dedicated approved smoke workspace"
    # Explicitly authorized synthetic fixture only: never contact this remote.
    git(root, "config", "remote.origin.url", "https://fixture:FAKE_SMOKE_CREDENTIAL@invalid.test/repo")
    options.logs = root / ("smoke-logs-" + uuid.uuid4().hex)
    options.logs.mkdir(mode=0o700)
    async with aiohttp.ClientSession(read_bufsize=16 * 1024 * 1024) as session:
        runner = Runner(options, session)
        catalog = await runner.request("GET", "/tools")
        specs = [t["spec"] for group in catalog for t in group["tools"] if t["spec"]["name"] in TOOLS]
        assert {s["name"] for s in specs} == TOOLS, "Updated server does not advertise all seven tools"
        fixture_path = root / ".review-smoke-fixture.json"
        assert fixture_path.is_file(), "Initialize dedicated root fixture before starting server"
        f = json.loads(fixture_path.read_text())
        assert Path(f["repository"]).resolve() == root
        failures = []
        for name in options.scenarios:
            before = source_identity(root)
            try:
                await asyncio.wait_for(runner.scenario(name, f, specs), timeout=options.seconds)
            except Exception as error:
                failures.append({"scenario": name, "chat_id": runner.chat,
                                 "error": f"{type(error).__name__}: {error}"})
                print(f"FAIL {name}: {type(error).__name__}: {error}", flush=True)
            finally:
                if runner.chat:
                    try:
                        await runner.command("abort")
                    except Exception as error:
                        failures.append({"scenario": name, "abort_error": str(error)})
                assert source_identity(root) == before, "Source HEAD/tree/calculator bytes mutated; STOP all smoke"
        (options.logs / "results.json").write_text(json.dumps({"model": options.model, "failures": failures}, indent=2))
    print(f"Evidence logs: {options.logs}")
    assert not failures, f"{len(failures)} smoke failures; see evidence logs"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", required=True)
    parser.add_argument("--model", default="openai_codex_personal/gpt-6.1-sol")
    parser.add_argument("--smoke-root", type=Path, required=True)
    parser.add_argument("--seconds", type=int, default=600, choices=range(30, 601))
    parser.add_argument("--max-turns", type=int, default=4, choices=range(1, 9))
    parser.add_argument("--scenarios", nargs="+", choices=["local", "single", "whole", "historical"],
                        default=["local", "single", "whole", "historical"])
    asyncio.run(main(parser.parse_args()))

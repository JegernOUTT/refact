"""Owned-engine fixture and strict runner for local utility endpoint integration tests."""
import argparse
import asyncio
from collections import Counter
import datetime
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace

import httpx
import pytest
import pytest_asyncio

TESTS = Path(__file__).resolve().parent
if str(TESTS) not in sys.path:
    sys.path.insert(0, str(TESTS))

from owned_engine import Engine, StubProvider, verified_copy, write_fixture

LOGS = TESTS.parents[2] / ".refact/verification/owned-utility"
TODAY = datetime.date.today().isoformat()
ALPHA_FILE = "src/utility_alpha_module.py"
BETA_FILE = "src/utility_beta_helper.rs"
ALPHA_BODY = "Alpha knowledge body mentions `UtilityAlphaEntity` and the alpha module.\nKG_ALPHA_BODY_MARKER"
BETA_BODY = "Beta decision body mentions `UtilityBetaEntity`.\nKG_BETA_BODY_MARKER"
GITHUB_TOOL = "cmdline_github"
GITHUB_ASK_RULE = "gh repo delete*"
GITHUB_DENY_RULE = "gh auth token*"
LANE_FILES = (
    "test03_at_commands_completion.py",
    "test12_tools_authorize_calls.py",
    "test_knowledge_graph_content_param.py",
    "test_knowledge_ops.py",
)
EXPECTED_NODEIDS = tuple(f"{module}::{name}" for module, names in (
    ("test03_at_commands_completion.py", (
        "test_bare_at_lists_registered_commands",
        "test_at_after_text_replaces_only_command_token",
        "test_partial_command_completes_to_file",
        "test_file_argument_completes_seeded_workspace_file",
        "test_unknown_command_and_plain_text_have_no_completion")),
    ("test12_tools_authorize_calls.py", (
        "test_paused_by_confirmation_and_denial_rules",
        "test_allowed_command_does_not_pause",
        "test_unparseable_arguments_are_denied",
        "test_missing_required_argument_is_not_authorized_by_rules",
        "test_malformed_request_body_is_rejected")),
    ("test_knowledge_graph_content_param.py", (
        "test_seeded_graph_structure",
        "test_knowledge_graph_without_content",
        "test_knowledge_graph_with_content_param_0",
        "test_knowledge_graph_with_content_param_1",
        "test_invalid_include_content_defaults_to_excluded",
        "test_knowledge_graph_response_size_difference")),
    ("test_knowledge_ops.py", (
        "test_update_memory",
        "test_update_memory_not_found",
        "test_update_rejects_invalid_status",
        "test_delete_memory_permanent",
        "test_delete_memory_archive",
        "test_delete_memory_not_found",
        "test_path_traversal_attack",
        "test_symlink_escape_attack",
        "test_non_markdown_file_rejected",
        "test_path_outside_knowledge_dir",
        "test_readonly_knowledge_dir_update_fails_without_mutation")),
) for name in names)
EXPECTED_PASSED = len(EXPECTED_NODEIDS)


def normalized_nodeid(nodeid):
    path, _, rest = nodeid.partition("::")
    return Path(path).name + "::" + rest


class StrictInventory:
    def __init__(self, expected=EXPECTED_NODEIDS):
        self.expected = Counter(expected)
        self.collected = []
        self.passed = []
        self.problems = []

    def pytest_collection_finish(self, session):
        self.collected = [normalized_nodeid(item.nodeid) for item in session.items]

    def pytest_collectreport(self, report):
        if report.skipped or report.failed:
            self.problems.append(f"collect {report.outcome}: {report.nodeid}")

    def pytest_runtest_logreport(self, report):
        nodeid = normalized_nodeid(report.nodeid)
        if hasattr(report, "wasxfail"):
            self.problems.append(f"xfail/xpass forbidden: {nodeid} ({report.when})")
        if report.skipped:
            self.problems.append(f"skipped: {nodeid} ({report.when})")
        if report.when == "call" and report.passed and not hasattr(report, "wasxfail"):
            self.passed.append(nodeid)

    def verdict(self):
        problems = list(self.problems)
        for label, seen in (("collected", Counter(self.collected)), ("passed", Counter(self.passed))):
            missing = sorted((self.expected - seen).elements())
            extra = sorted((seen - self.expected).elements())
            if missing:
                problems.append(f"{label} missing: {missing}")
            if extra:
                problems.append(f"{label} unexpected or duplicate: {extra}")
        return problems


def inventory_self_check():
    def report(nodeid, when="call", outcome="passed", **extra):
        return SimpleNamespace(nodeid=nodeid, when=when, outcome=outcome, passed=outcome == "passed",
                               failed=outcome == "failed", skipped=outcome == "skipped", **extra)

    def run(expected, collected, reports):
        inventory = StrictInventory(expected)
        inventory.pytest_collection_finish(SimpleNamespace(items=[SimpleNamespace(nodeid=n) for n in collected]))
        for item in reports:
            inventory.pytest_runtest_logreport(item)
        return inventory.verdict()

    good = ["tests/a.py::t1", "tests/a.py::t2"]
    expected = ("a.py::t1", "a.py::t2")
    cases = {
        "exact": (good, [report(n) for n in good], False),
        "substitution": (["tests/a.py::t1", "tests/a.py::t3"],
                         [report("tests/a.py::t1"), report("tests/a.py::t3")], True),
        "duplicate": (good + ["tests/a.py::t1"], [report(n) for n in good] + [report("tests/a.py::t1")], True),
        "missing": (good, [report("tests/a.py::t1")], True),
        "xpass": (good, [report("tests/a.py::t1"), report("tests/a.py::t2", wasxfail="")], True),
        "xfail": (good, [report("tests/a.py::t1"), report("tests/a.py::t2", outcome="skipped", wasxfail="r")], True),
        "skip": (good, [report("tests/a.py::t1"), report("tests/a.py::t2", when="setup", outcome="skipped")], True),
    }
    results = {}
    for name, (collected, reports, must_fail) in cases.items():
        problems = run(expected, collected, reports)
        assert bool(problems) == must_fail, (name, problems)
        results[name] = problems
    assert len(set(EXPECTED_NODEIDS)) == len(EXPECTED_NODEIDS) == 27
    assert {n.split("::")[0] for n in EXPECTED_NODEIDS} == set(LANE_FILES)
    return results


class UtilityEngine:
    def __init__(self, engine, workspace, config):
        self.engine = engine
        self.workspace = workspace
        self.config = config
        self.knowledge = workspace / ".refact" / "knowledge"
        self.base_url = f"http://127.0.0.1:{engine.port}"
        self.github_config = workspace / ".refact" / "integrations.d" / f"{GITHUB_TOOL}.yaml"

    def client(self):
        return httpx.AsyncClient(base_url=self.base_url + "/v1", timeout=20, trust_env=False)


def memory_text(identifier, title, tags, kind, filenames, links, body):
    quoted = lambda items: "[" + ", ".join(json.dumps(item) for item in items) + "]"
    return ("---\n"
            f"id: {json.dumps(identifier)}\n"
            f"title: {json.dumps(title)}\n"
            f"tags: {quoted(tags)}\n"
            f"kind: {kind}\n"
            f"created: {TODAY}\n"
            f"updated: {TODAY}\n"
            f"filenames: {quoted(filenames)}\n"
            f"links: {quoted(links)}\n"
            "status: active\n"
            "---\n\n" + body + "\n")


def seed_workspace(workspace: Path):
    (workspace / "src").mkdir()
    (workspace / ALPHA_FILE).write_text("def utility_alpha():\n    return 'alpha'\n")
    (workspace / BETA_FILE).write_text("pub fn utility_beta() -> u32 { 7 }\n")
    (workspace / "outside.md").write_text("WORKSPACE_OUTSIDE_KNOWLEDGE\n")
    knowledge = workspace / ".refact" / "knowledge"
    knowledge.mkdir(parents=True)
    (knowledge / "alpha.md").write_text(memory_text(
        "kg-alpha", "Utility Alpha Knowledge", ["domain:utility-alpha", "component:utility-shared"],
        "code", [ALPHA_FILE], ["kg-beta"], ALPHA_BODY))
    (knowledge / "beta.md").write_text(memory_text(
        "kg-beta", "Utility Beta Decision", ["domain:utility-beta", "component:utility-shared"],
        "decision", [BETA_FILE], [], BETA_BODY))
    (workspace / ".refact" / "integrations.d" / f"{GITHUB_TOOL}.yaml").write_text(
        "command: \"gh %command%\"\n"
        f"command_workdir: {json.dumps(str(workspace))}\n"
        "description: Local GitHub CLI confirmation-rule fixture, never executed by these tests\n"
        "parameters:\n"
        "  - {name: command, type: string, description: gh subcommand}\n"
        "  - {name: project_dir, type: string, description: project directory}\n"
        "parameters_required: [command]\n"
        "timeout: '10'\n"
        "available: {on_your_laptop: true, when_isolated: true}\n"
        f"confirmation: {{ask_user: [{json.dumps(GITHUB_ASK_RULE)}], deny: [{json.dumps(GITHUB_DENY_RULE)}]}}\n")


async def until(label, callback, seconds=60):
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        last = await callback()
        if last:
            return last
        await asyncio.sleep(.2)
    raise AssertionError(f"{label}: finite timeout")


@pytest_asyncio.fixture
async def utility_engine(request, tmp_path):
    binary = Path(os.environ["REFACT_TEST_BINARY"]).resolve(strict=True)
    expected = os.environ["REFACT_TEST_BINARY_SHA256"]
    copied = verified_copy(binary, expected, tmp_path / "refact-lsp")
    workspace, config, cache = (tmp_path / name for name in ("workspace", "config", "cache"))
    provider = StubProvider(workspace / "release", workspace / "provider-invocations", tool_call=False)
    engine = None
    try:
        await provider.start()
        write_fixture(workspace, config, cache, provider.port)
        seed_workspace(workspace)
        engine = Engine(copied, workspace, cache, config)
        engine.start()
        owned = UtilityEngine(engine, workspace, config)
        async with owned.client() as client:
            ping = await client.get("/ping")
            assert ping.status_code == 200, ping.text
        yield owned
    finally:
        try:
            if engine:
                engine.stop()
                LOGS.mkdir(parents=True, exist_ok=True)
                if getattr(engine, "log_path", None) and engine.log_path.exists():
                    shutil.copyfile(engine.log_path, LOGS / (request.node.name + ".log"))
                (LOGS / (request.node.name + ".identity.json")).write_text(json.dumps({
                    "binary_sha256": expected, "pid": engine.process.pid if engine.process else None,
                    "port": engine.port, "workspace": str(workspace)}))
        finally:
            await provider.stop()
            copied.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--binary-sha256")
    parser.add_argument("--self-check", action="store_true")
    parser.add_argument("--evidence-prefix", default="")
    args = parser.parse_args()
    checks = inventory_self_check()
    if args.self_check:
        print(json.dumps({"inventory_self_check": "passed", "cases": checks}), flush=True)
        return
    if not args.binary or not args.binary_sha256:
        parser.error("--binary and --binary-sha256 are required")
    prefix = args.evidence_prefix
    LOGS.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="owned-utility-run-") as directory:
        binary = verified_copy(args.binary.resolve(strict=True), args.binary_sha256,
                               Path(directory) / "refact-lsp")
        os.environ["REFACT_TEST_BINARY"] = str(binary)
        os.environ["REFACT_TEST_BINARY_SHA256"] = args.binary_sha256
        identity = subprocess.run([str(binary), "--version"], check=True,
                                  capture_output=True, text=True).stdout.strip()
        results = StrictInventory()
        code = pytest.main(["-q", "--tb=short", "-p", "no:cacheprovider",
                            "--junitxml=" + str(LOGS / (prefix + "results.xml")),
                            *[str(TESTS / name) for name in LANE_FILES]], plugins=[results])
        problems = results.verdict()
        summary = {"binary_sha256": args.binary_sha256, "version": identity,
                   "passed": len(results.passed), "expected_passed": EXPECTED_PASSED,
                   "inventory_problems": problems, "passed_nodeids": sorted(results.passed),
                   "pytest_exit": int(code)}
        (LOGS / (prefix + "summary.json")).write_text(json.dumps(summary, indent=2) + "\n")
        print(json.dumps(summary), flush=True)
        raise SystemExit(int(code) or int(bool(problems)))


if __name__ == "__main__":
    main()

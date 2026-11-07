import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys


def digest(data):
    return hashlib.sha256(data).hexdigest()


def run(command, root, log):
    result = subprocess.run(
        command,
        cwd=root / "refact-agent/engine",
        env=dict(os.environ, CARGO_TERM_COLOR="never", RUST_MIN_STACK="16777216"),
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=900,
    )
    log.write_bytes(result.stdout)
    return result.returncode, result.stdout.decode("utf-8", errors="replace")


def main():
    if sys.platform != "win32":
        raise RuntimeError("Windows execution is required for artifact qualification")
    root = Path(__file__).resolve().parents[2]
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    expected = os.environ.get("BROWSER_QUALIFICATION_SHA")
    if not expected or revision != expected:
        raise RuntimeError("Qualification revision does not match the expected source")
    output = root / "refact-agent/engine/target/browser-artifact-witness"
    output.mkdir(parents=True, exist_ok=True)
    store = root / "refact-agent/engine/crates/refact-browser/src/artifact_store.rs"
    startup = root / "refact-agent/engine/vendor/headless_chrome/src/browser/process.rs"
    cases = [
        (
            "owner_acl",
            store,
            "O:{sid_string}D:P(A;OICI;FA;;;{sid_string})",
            "O:{sid_string}D:P(A;OICI;FA;;;WD)",
            1,
            "refact-browser",
            "artifact_store::windows_platform::tests::windows_private_acl_is_protected_owner_only",
            "directory ACL not owner-only",
        ),
        (
            "handle_sharing",
            store,
            "let readonly = reopen(&entry.file, GENERIC_READ, FILE_SHARE_READ)?;",
            "let readonly = reopen(&entry.file, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)?;",
            1,
            "refact-browser",
            "artifact_store::windows_platform::tests::windows_pinned_handles_prevent_substitution",
            "std::fs::remove_file(&artifact.path).is_err()",
        ),
        (
            "path_reader_compatibility",
            store,
            ".and_then(|_| publish_read_handle(entry));",
            ".and_then(|_| Ok(()));",
            1,
            "refact-browser",
            "artifact_store::windows_platform::tests::windows_pinned_handles_prevent_substitution",
            "published artifact must support read-only path consumers",
        ),
        (
            "blocked_expired_read",
            store,
            "if now.saturating_duration_since(entry.last) >= TTL {",
            "if false {",
            1,
            "refact-browser",
            "artifact_store::windows_platform::tests::windows_quota_lru_and_idle_ttl",
            "store.read_report(&blocked.id, 0, 16).is_err()",
        ),
        (
            "reparse_refusal",
            store,
            "if value.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {",
            "if false {",
            1,
            "refact-browser",
            "artifact_store::windows_platform::tests::windows_existing_junction_root_is_rejected",
            "ArtifactStore::new(&junction).is_err()",
        ),
        (
            "pending_startup_pipe",
            startup,
            "let count = read_ready_stderr(stderr, &mut buffer)?;",
            "let count = Some(stderr.read(&mut buffer)?);",
            1,
            "headless_chrome",
            "browser::process::tests::pending_stderr_obeys_original_startup_deadline",
            "typed startup timeout",
        ),
    ]
    receipts = []
    try:
        for name, path, before, after, count, package, test, oracle in cases:
            saved = path.read_bytes()
            text = saved.decode("utf-8")
            if text.count(before) != count:
                raise RuntimeError(f"{name}: expected {count} production mutation anchors")
            mutant = text.replace(before, after).encode("utf-8")
            command = [
                "cargo", "test", "--locked", "--profile", "ci-test", "-p", package,
                "--lib", test, "--", "--exact", "--nocapture", "--test-threads=1",
            ]
            receipt = {
                "name": name,
                "revision": revision,
                "expected_sha": expected,
                "source": path.relative_to(root).as_posix(),
                "mutant_sha256": digest(mutant),
                "restored_sha256": digest(saved),
                "command": command,
                "oracle": oracle,
            }
            receipts.append(receipt)
            try:
                path.write_bytes(mutant)
                code, result = run(command, root, output / f"{name}-mutant.log")
                receipt["mutant_exit_code"] = code
                if code != 101 or oracle not in result or test not in result or "test result: FAILED. 0 passed; 1 failed" not in result:
                    raise RuntimeError(f"{name}: mutant did not fail the intended runtime oracle")
            finally:
                path.write_bytes(saved)
                if digest(path.read_bytes()) != receipt["restored_sha256"]:
                    raise RuntimeError(f"{name}: production source was not restored exactly")
            code, result = run(command, root, output / f"{name}-restored.log")
            receipt["restored_exit_code"] = code
            if code != 0 or "1 passed; 0 failed" not in result:
                raise RuntimeError(f"{name}: restored production test did not pass")
            print(json.dumps(receipt), flush=True)
        subprocess.run(["git", "diff", "--exit-code"], cwd=root, check=True)
    finally:
        (output / "receipts.json").write_text(json.dumps(receipts, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

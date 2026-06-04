#!/usr/bin/env python3
import argparse
import json
import re
import sys
import tempfile
from collections import Counter, defaultdict
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

FINDING_CODES = [
    "BAD_REPORT_CONTENT",
    "REPORT_SAYS_REPLACED",
    "SOURCE_PRESERVING_REPORT_LACKS_SOURCE_IDS",
    "INTERNAL_SUMMARY_UNPAIRED",
    "ZERO_SAVINGS_APPLIED",
    "REPORT_SUMMARY_PAIR_MISMATCH",
    "SOURCE_PRESERVING_SOURCE_MISSING",
    "LENGTH_STOP_WITHOUT_COMPRESSION",
    "REPEATED_COMPRESSION_NO_SAVINGS",
    "MALFORMED_JSON",
]

SECRET_PATTERNS = [
    re.compile(r"sk-[A-Za-z0-9_\-]{8,}"),
    re.compile(r"(?i)(bearer\s+)[A-Za-z0-9._\-]{8,}"),
    re.compile(r"(?i)(api[_-]?key\s*[:=]\s*)[^\s,;\]}]+"),
    re.compile(r"(?i)(token\s*[:=]\s*)[^\s,;\]}]+"),
    re.compile(r"(?i)(authorization\s*[:=]\s*)[^\n,;]+"),
]

LENGTH_STOP_RE = re.compile(
    r"(?i)(context_length_exceeded|context too large|maximum context|provider length|finish_reason[\"': ]+length|stop reason[\"': ]+length)"
)


@dataclass
class Example:
    path: str
    index: int | None
    preview: str


@dataclass
class ScanResult:
    counts: Counter
    examples: dict[str, list[Example]]
    files_scanned: int
    files_considered: int


def redact(text: str) -> str:
    redacted = text
    for pattern in SECRET_PATTERNS:
        if pattern.pattern.lower().startswith("(?i)(bearer"):
            redacted = pattern.sub(r"\1[REDACTED]", redacted)
        elif "api" in pattern.pattern.lower() or "token" in pattern.pattern.lower() or "authorization" in pattern.pattern.lower():
            redacted = pattern.sub(r"\1[REDACTED]", redacted)
        else:
            redacted = pattern.sub("[REDACTED_SECRET]", redacted)
    return redacted


def preview(value: Any, limit: int = 180) -> str:
    if isinstance(value, str):
        text = value
    else:
        try:
            text = json.dumps(value, ensure_ascii=False, sort_keys=True)
        except TypeError:
            text = str(value)
    text = redact(" ".join(text.split()))
    if len(text) > limit:
        return text[: limit - 1].rstrip() + "…"
    return text


def is_record(value: Any) -> bool:
    return isinstance(value, dict)


def content_text(message: dict[str, Any]) -> str:
    content = message.get("content")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts: list[str] = []
        for item in content:
            if isinstance(item, str):
                parts.append(item)
            elif is_record(item):
                for key in ("text", "m_content", "file_content", "content"):
                    if isinstance(item.get(key), str):
                        parts.append(item[key])
                        break
        return "\n".join(parts)
    return ""


def nested_record(message: dict[str, Any], key: str) -> dict[str, Any] | None:
    direct = message.get(key)
    if is_record(direct):
        return direct
    extra = message.get("extra")
    if is_record(extra) and is_record(extra.get(key)):
        return extra[key]
    return None


def compression_report(message: dict[str, Any]) -> dict[str, Any] | None:
    meta = nested_record(message, "compression_report")
    if is_record(meta) and meta.get("kind") == "chat_compression_report":
        return meta
    return None


def summary_compression(message: dict[str, Any]) -> dict[str, Any] | None:
    meta = nested_record(message, "compression")
    if is_record(meta) and meta.get("kind") == "llm_segment_summary":
        return meta
    return None


def event_metadata(message: dict[str, Any]) -> dict[str, Any] | None:
    meta = nested_record(message, "event")
    return meta if is_record(meta) else None


def source_ids(meta: dict[str, Any]) -> list[str]:
    for key in ("source_message_ids", "summarized_source_message_ids"):
        value = meta.get(key)
        if isinstance(value, list):
            return [item for item in value if isinstance(item, str) and item]
    return []


def message_id(message: dict[str, Any]) -> str | None:
    value = message.get("message_id") or message.get("id")
    return value if isinstance(value, str) and value else None


def messages_from_json(data: Any) -> list[dict[str, Any]]:
    if is_record(data) and isinstance(data.get("messages"), list):
        return [msg for msg in data["messages"] if is_record(msg)]
    if is_record(data) and is_record(data.get("thread")) and isinstance(data["thread"].get("messages"), list):
        return [msg for msg in data["thread"]["messages"] if is_record(msg)]
    return []


def iter_trajectory_files(root: Path) -> list[Path]:
    files: set[Path] = set()
    global_dir = root / "trajectories"
    if global_dir.is_dir():
        files.update(global_dir.glob("*.json"))
    tasks_dir = root / "tasks"
    if tasks_dir.is_dir():
        files.update(tasks_dir.glob("*/trajectories/**/*.json"))
        files.update(tasks_dir.glob("*/trajectories/*.json"))
    return sorted(files)


def within_since(path: Path, since_days: float | None) -> bool:
    if since_days is None:
        return True
    cutoff = datetime.now(timezone.utc) - timedelta(days=since_days)
    try:
        mtime = datetime.fromtimestamp(path.stat().st_mtime, timezone.utc)
    except OSError:
        return False
    return mtime >= cutoff


def relative_path(path: Path, root: Path) -> str:
    try:
        return path.relative_to(root).as_posix()
    except ValueError:
        return path.as_posix()


def has_matching_pair(report_meta: dict[str, Any] | None, summary_meta: dict[str, Any] | None) -> bool:
    if not report_meta or not summary_meta:
        return False
    report_hash = report_meta.get("source_hash")
    summary_hash = summary_meta.get("source_hash")
    hashes_match = (
        isinstance(report_hash, str)
        and bool(report_hash)
        and isinstance(summary_hash, str)
        and report_hash == summary_hash
    )
    report_ids = source_ids(report_meta)
    summary_ids = source_ids(summary_meta)
    ids_match = bool(report_ids) and report_ids == summary_ids
    return hashes_match or ids_match


def report_requires_adjacent_summary(report_meta: dict[str, Any], next_summary: dict[str, Any] | None) -> bool:
    return (
        report_meta.get("compression_kind") == "llm_segment_summary"
        or report_meta.get("insert_mode") == "source_preserving"
        or next_summary is not None
    )


def compression_attempt_after(messages: list[dict[str, Any]], start: int) -> bool:
    for msg in messages[start + 1 :]:
        if compression_report(msg) or summary_compression(msg):
            return True
        event = event_metadata(msg)
        if event and event.get("source") == "chat.summarizer":
            return True
        text = content_text(msg).lower()
        if "context compression" in text or "chat context compressed" in text:
            return True
    return False


def add_finding(
    counts: Counter,
    examples: dict[str, list[Example]],
    code: str,
    path: str,
    index: int | None,
    value: Any,
    max_examples: int,
) -> None:
    counts[code] += 1
    if len(examples[code]) < max_examples:
        examples[code].append(Example(path, index, preview(value)))


def scan_messages(
    messages: list[dict[str, Any]],
    relpath: str,
    counts: Counter,
    examples: dict[str, list[Example]],
    max_examples: int,
) -> None:
    ids = {mid for msg in messages if (mid := message_id(msg))}
    nonpositive_reports = 0

    for idx, msg in enumerate(messages):
        report = compression_report(msg)
        summary = summary_compression(msg)
        text = content_text(msg)

        if report:
            lowered = text.lower()
            if "summary kept for the model" in lowered:
                add_finding(counts, examples, "BAD_REPORT_CONTENT", relpath, idx, text, max_examples)
            if "replaced" in lowered:
                add_finding(counts, examples, "BAD_REPORT_CONTENT", relpath, idx, text, max_examples)
                add_finding(counts, examples, "REPORT_SAYS_REPLACED", relpath, idx, text, max_examples)

            if report.get("insert_mode") == "source_preserving" and not source_ids(report):
                add_finding(
                    counts,
                    examples,
                    "SOURCE_PRESERVING_REPORT_LACKS_SOURCE_IDS",
                    relpath,
                    idx,
                    report,
                    max_examples,
                )

            tokens_before = report.get("tokens_before")
            tokens_after = report.get("tokens_after")
            saved = report.get("estimated_tokens_saved")
            reduction = report.get("reduction_percent")
            nonpositive = (
                isinstance(saved, (int, float)) and saved <= 0
            ) or (
                isinstance(tokens_before, (int, float))
                and isinstance(tokens_after, (int, float))
                and tokens_after >= tokens_before
            ) or (isinstance(reduction, (int, float)) and reduction <= 0)
            if nonpositive:
                nonpositive_reports += 1
                add_finding(counts, examples, "ZERO_SAVINGS_APPLIED", relpath, idx, report, max_examples)

            next_summary = summary_compression(messages[idx + 1]) if idx + 1 < len(messages) else None
            if report_requires_adjacent_summary(report, next_summary) and not has_matching_pair(report, next_summary):
                add_finding(
                    counts,
                    examples,
                    "REPORT_SUMMARY_PAIR_MISMATCH",
                    relpath,
                    idx,
                    report,
                    max_examples,
                )

            missing = [sid for sid in source_ids(report) if sid not in ids]
            if report.get("insert_mode") == "source_preserving" and missing:
                add_finding(
                    counts,
                    examples,
                    "SOURCE_PRESERVING_SOURCE_MISSING",
                    relpath,
                    idx,
                    {"missing_source_ids": missing, "report": report},
                    max_examples,
                )

        if summary:
            prior_report = compression_report(messages[idx - 1]) if idx > 0 else None
            if not has_matching_pair(prior_report, summary):
                add_finding(counts, examples, "INTERNAL_SUMMARY_UNPAIRED", relpath, idx, summary, max_examples)
                add_finding(
                    counts,
                    examples,
                    "REPORT_SUMMARY_PAIR_MISMATCH",
                    relpath,
                    idx,
                    summary,
                    max_examples,
                )
            missing = [sid for sid in source_ids(summary) if sid not in ids]
            if summary.get("insert_mode") == "source_preserving" and missing:
                add_finding(
                    counts,
                    examples,
                    "SOURCE_PRESERVING_SOURCE_MISSING",
                    relpath,
                    idx,
                    {"missing_source_ids": missing, "summary": summary},
                    max_examples,
                )

        finish_reason = msg.get("finish_reason")
        if finish_reason == "length" or LENGTH_STOP_RE.search(text):
            if not compression_attempt_after(messages, idx):
                add_finding(
                    counts,
                    examples,
                    "LENGTH_STOP_WITHOUT_COMPRESSION",
                    relpath,
                    idx,
                    text or {"finish_reason": finish_reason},
                    max_examples,
                )

    if nonpositive_reports >= 2:
        add_finding(
            counts,
            examples,
            "REPEATED_COMPRESSION_NO_SAVINGS",
            relpath,
            None,
            {"nonpositive_reports": nonpositive_reports},
            max_examples,
        )


def scan_root(root: Path, since_days: float | None, max_examples: int = 5) -> ScanResult:
    counts: Counter = Counter({code: 0 for code in FINDING_CODES})
    examples: dict[str, list[Example]] = defaultdict(list)
    files = iter_trajectory_files(root)
    considered = 0
    scanned = 0

    for path in files:
        if not within_since(path, since_days):
            continue
        considered += 1
        relpath = relative_path(path, root)
        try:
            with path.open("r", encoding="utf-8") as fh:
                data = json.load(fh)
        except (OSError, json.JSONDecodeError) as exc:
            add_finding(counts, examples, "MALFORMED_JSON", relpath, None, str(exc), max_examples)
            continue
        messages = messages_from_json(data)
        if not messages:
            continue
        scanned += 1
        scan_messages(messages, relpath, counts, examples, max_examples)

    return ScanResult(counts, examples, scanned, considered)


def print_result(result: ScanResult) -> None:
    print(f"FILES_CONSIDERED: {result.files_considered}")
    print(f"FILES_SCANNED: {result.files_scanned}")
    for code in FINDING_CODES:
        print(f"{code}: {result.counts[code]}")
    for code in FINDING_CODES:
        items = result.examples.get(code, [])
        if not items:
            continue
        print(f"\n{code} examples:")
        for item in items:
            idx = "-" if item.index is None else str(item.index)
            print(f"  - {item.path}:{idx}: {item.preview}")


def write_json(path: Path, messages: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"messages": messages}, indent=2), encoding="utf-8")


def self_test() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp) / ".refact"
        write_json(
            root / "trajectories" / "bad.json",
            [
                {"role": "user", "message_id": "u1", "content": "go"},
                {"role": "assistant", "message_id": "a1", "content": "source"},
                {
                    "role": "compression_report",
                    "content": "Summary kept for the model and replaced old content sk-test-secret123",
                    "extra": {
                        "compression_report": {
                            "kind": "chat_compression_report",
                            "insert_mode": "source_preserving",
                            "source_hash": "h1",
                            "source_message_ids": [],
                            "tokens_before": 10,
                            "tokens_after": 10,
                            "estimated_tokens_saved": 0,
                            "reduction_percent": 0,
                        }
                    },
                },
                {
                    "role": "assistant",
                    "content": "internal summary",
                    "extra": {
                        "compression": {
                            "kind": "llm_segment_summary",
                            "insert_mode": "source_preserving",
                            "source_hash": "different",
                            "source_message_ids": ["missing-source"],
                        }
                    },
                },
                {"role": "assistant", "content": "stopped", "finish_reason": "length"},
            ],
        )
        write_json(
            root / "trajectories" / "lone-summary.json",
            [
                {"role": "user", "message_id": "u2", "content": "hello"},
                {
                    "role": "assistant",
                    "message_id": "s1",
                    "content": "hidden summary",
                    "extra": {
                        "compression": {
                            "kind": "llm_segment_summary",
                            "insert_mode": "source_preserving",
                            "source_hash": "h2",
                            "source_message_ids": ["u2"],
                        }
                    },
                },
            ],
        )
        write_json(
            root / "trajectories" / "unidentifiable-pair.json",
            [
                {"role": "user", "message_id": "u3", "content": "hello"},
                {
                    "role": "compression_report",
                    "content": "Context compression applied",
                    "extra": {
                        "compression_report": {
                            "kind": "chat_compression_report",
                            "insert_mode": "source_preserving",
                            "tokens_before": 100,
                            "tokens_after": 50,
                            "estimated_tokens_saved": 50,
                            "reduction_percent": 50,
                        }
                    },
                },
                {
                    "role": "assistant",
                    "content": "internal summary",
                    "extra": {
                        "compression": {
                            "kind": "llm_segment_summary",
                            "insert_mode": "source_preserving",
                        }
                    },
                },
            ],
        )
        write_json(
            root / "trajectories" / "deterministic-report-only.json",
            [
                {"role": "user", "message_id": "u4", "content": "hello"},
                {
                    "role": "compression_report",
                    "content": "Chat context compressed deterministically",
                    "extra": {
                        "compression_report": {
                            "kind": "chat_compression_report",
                            "insert_mode": "deterministic",
                            "tokens_before": 100,
                            "tokens_after": 60,
                            "estimated_tokens_saved": 40,
                            "reduction_percent": 40,
                        }
                    },
                },
            ],
        )
        result = scan_root(root, since_days=3, max_examples=10)
        required = [
            "BAD_REPORT_CONTENT",
            "REPORT_SAYS_REPLACED",
            "SOURCE_PRESERVING_REPORT_LACKS_SOURCE_IDS",
            "INTERNAL_SUMMARY_UNPAIRED",
            "ZERO_SAVINGS_APPLIED",
            "REPORT_SUMMARY_PAIR_MISMATCH",
            "SOURCE_PRESERVING_SOURCE_MISSING",
            "LENGTH_STOP_WITHOUT_COMPRESSION",
        ]
        missing = [code for code in required if result.counts[code] <= 0]
        if missing:
            print_result(result)
            print(f"SELF_TEST_FAILED missing={missing}", file=sys.stderr)
            return 1
        if result.counts["INTERNAL_SUMMARY_UNPAIRED"] < 2:
            print_result(result)
            print("SELF_TEST_FAILED lone summary was not detected", file=sys.stderr)
            return 1
        pair_mismatch_examples = result.examples.get("REPORT_SUMMARY_PAIR_MISMATCH", [])
        if any(item.path.endswith("deterministic-report-only.json") for item in pair_mismatch_examples):
            print_result(result)
            print("SELF_TEST_FAILED deterministic report-only artifact was flagged as pair mismatch", file=sys.stderr)
            return 1
        unidentifiable_pair_count = sum(
            1
            for item in pair_mismatch_examples
            if item.path.endswith("unidentifiable-pair.json")
        )
        if unidentifiable_pair_count < 2:
            print_result(result)
            print("SELF_TEST_FAILED unidentifiable report/summary pair was not detected", file=sys.stderr)
            return 1
        rendered = result.examples["BAD_REPORT_CONTENT"][0].preview
        if "sk-test-secret123" in rendered:
            print("SELF_TEST_FAILED secret leaked", file=sys.stderr)
            return 1
        print("SELF_TEST_OK")
        print_result(result)
        return 0


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Scan local .refact trajectories/tasks for bad compression artifacts."
    )
    parser.add_argument("root", nargs="?", default=".refact", help="Path to the .refact directory")
    parser.add_argument("--since-days", type=float, default=None, help="Only scan files modified in the last N days")
    parser.add_argument("--fail-on-findings", action="store_true", help="Return non-zero when any finding is present")
    parser.add_argument("--max-examples", type=int, default=5, help="Maximum examples to print per finding")
    parser.add_argument("--self-test", action="store_true", help="Run a synthetic temp-dir scanner test")
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    if args.self_test:
        return self_test()
    root = Path(args.root).resolve()
    result = scan_root(root, args.since_days, max(0, args.max_examples))
    print_result(result)
    total_findings = sum(result.counts.values())
    if args.fail_on_findings and total_findings > 0:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))

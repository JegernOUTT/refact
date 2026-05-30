import argparse
import json
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from typing import Any


JsonObject = dict[str, Any]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Read-only verifier for Buddy live chat reactions. Polls /v1/buddy, "
            "prints recent chat_reaction_debug attempts, checks whether the latest "
            "emitted reaction is still present in runtime_queue, and states whether "
            "BuddyChatCompanion should render the speech bubble for the active chat."
        )
    )
    parser.add_argument(
        "--port",
        type=int,
        default=8001,
        help="Local refact-lsp HTTP port for /v1/buddy (default: 8001).",
    )
    parser.add_argument(
        "--watch",
        type=float,
        default=0,
        help="Poll for this many seconds before evaluating the latest emitted attempt.",
    )
    parser.add_argument(
        "--chat-id",
        help="Only consider emitted Buddy chat reactions for this chat id.",
    )
    parser.add_argument(
        "--expect-visible",
        action="store_true",
        help=(
            "Exit non-zero when no recent emitted attempt is found in the watch window, "
            "when the latest emitted attempt is for a different chat id, or when its "
            "event id is missing from runtime_queue while still within TTL."
        ),
    )
    return parser.parse_args()


def parse_timestamp(value: Any) -> datetime | None:
    if not isinstance(value, str) or not value.strip():
        return None
    text = value.strip()
    if text.endswith("Z"):
        text = f"{text[:-1]}+00:00"
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def fetch_snapshot(port: int) -> JsonObject:
    url = f"http://127.0.0.1:{port}/v1/buddy"
    request = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(request, timeout=5) as response:
        body = response.read().decode("utf-8")
    payload = json.loads(body)
    if not isinstance(payload, dict):
        raise RuntimeError("/v1/buddy did not return a JSON object")
    return payload


def as_list(value: Any) -> list[Any]:
    return value if isinstance(value, list) else []


def as_dict(value: Any) -> JsonObject:
    return value if isinstance(value, dict) else {}


def value_or_dash(value: Any) -> str:
    if value is None:
        return "-"
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


def format_ms(value: float | None) -> str:
    if value is None:
        return "unknown"
    if value < 0:
        return "expired"
    seconds = value / 1000
    if seconds < 60:
        return f"{seconds:.1f}s"
    return f"{seconds / 60:.1f}m"


def event_age_ms(event: JsonObject, now: datetime) -> float | None:
    created_at = parse_timestamp(event.get("created_at"))
    if created_at is None:
        return None
    return max(0.0, (now - created_at).total_seconds() * 1000)


def attempt_age_ms(attempt: JsonObject, now: datetime) -> float | None:
    attempted_at = parse_timestamp(attempt.get("attempted_at"))
    if attempted_at is None:
        return None
    return max(0.0, (now - attempted_at).total_seconds() * 1000)


def ttl_remaining_ms(event: JsonObject | None, attempt: JsonObject, now: datetime) -> float | None:
    ttl = event.get("ttl_ms") if event else attempt.get("ttl_ms")
    if not isinstance(ttl, (int, float)) or ttl <= 0:
        return None
    origin = parse_timestamp(event.get("created_at")) if event else None
    if origin is None:
        origin = parse_timestamp(attempt.get("attempted_at"))
    if origin is None:
        return None
    return ttl - (now - origin).total_seconds() * 1000


def is_within_ttl(event: JsonObject | None, attempt: JsonObject, now: datetime) -> bool:
    remaining = ttl_remaining_ms(event, attempt, now)
    if remaining is None:
        return True
    return remaining > 0


def safe_runtime_text(event: JsonObject | None) -> str:
    if not event:
        return "missing"
    speech_text = event.get("speech_text")
    if isinstance(speech_text, str) and speech_text.strip():
        return speech_text.strip()
    title = event.get("title")
    if isinstance(title, str) and title.strip():
        return f"present title={title.strip()}"
    return "present"


def recent_attempts(snapshot: JsonObject) -> list[JsonObject]:
    debug = as_dict(snapshot.get("chat_reaction_debug"))
    attempts = []
    for item in as_list(debug.get("recent_attempts")):
        if isinstance(item, dict):
            attempts.append(item)
    return attempts


def emitted_attempts(snapshot: JsonObject) -> list[JsonObject]:
    return [
        attempt
        for attempt in recent_attempts(snapshot)
        if attempt.get("result") == "emitted" and attempt.get("event_id")
    ]


def sort_key_for_attempt(attempt: JsonObject) -> float:
    parsed = parse_timestamp(attempt.get("attempted_at"))
    return parsed.timestamp() if parsed else 0


def latest_emitted(snapshot: JsonObject, chat_id: str | None) -> tuple[JsonObject | None, JsonObject | None]:
    emitted = sorted(emitted_attempts(snapshot), key=sort_key_for_attempt)
    if not emitted:
        return None, None
    latest_any = emitted[-1]
    if chat_id is None:
        return latest_any, latest_any
    filtered = [attempt for attempt in emitted if attempt.get("chat_id") == chat_id]
    if not filtered:
        return None, latest_any
    return filtered[-1], latest_any


def runtime_queue(snapshot: JsonObject) -> list[JsonObject]:
    events = []
    for item in as_list(snapshot.get("runtime_queue")):
        if isinstance(item, dict):
            events.append(item)
    return events


def find_runtime_queue_event(snapshot: JsonObject, event_id: Any) -> JsonObject | None:
    if not isinstance(event_id, str):
        return None
    for event in runtime_queue(snapshot):
        if event.get("id") == event_id:
            return event
    return None


def find_now_playing_event(snapshot: JsonObject, event_id: Any) -> JsonObject | None:
    if not isinstance(event_id, str):
        return None
    now_playing = snapshot.get("now_playing")
    if isinstance(now_playing, dict) and now_playing.get("id") == event_id:
        return now_playing
    return None


def print_settings_summary(snapshot: JsonObject) -> None:
    settings = as_dict(snapshot.get("settings"))
    observers = as_dict(settings.get("observers"))
    identity = as_dict(as_dict(snapshot.get("state")).get("identity"))
    print("Buddy settings summary")
    print(f"  enabled: {value_or_dash(snapshot.get('enabled'))}")
    print(f"  identity: {value_or_dash(identity.get('name'))}")
    print(f"  settings.enabled: {value_or_dash(settings.get('enabled'))}")
    print(f"  chat_reactions_enabled: {value_or_dash(settings.get('chat_reactions_enabled'))}")
    print(f"  message_observation_enabled: {value_or_dash(settings.get('message_observation_enabled'))}")
    print(f"  humor_enabled: {value_or_dash(settings.get('humor_enabled'))}")
    print(f"  quiet_mode: {value_or_dash(settings.get('quiet_mode'))}")
    print(f"  observer.chat_pattern: {value_or_dash(observers.get('chat_pattern'))}")


def print_recent_attempts(snapshot: JsonObject) -> None:
    debug = as_dict(snapshot.get("chat_reaction_debug"))
    attempts = recent_attempts(snapshot)
    print("chat_reaction_debug")
    print(f"  counts_by_result: {json.dumps(as_dict(debug.get('counts_by_result')), sort_keys=True)}")
    print(f"  last_skip_reason: {value_or_dash(debug.get('last_skip_reason'))}")
    print(f"  last_emitted_at: {value_or_dash(debug.get('last_emitted_at'))}")
    if not attempts:
        print("  recent_attempts: none")
        return
    print("  recent_attempts:")
    for attempt in attempts[-10:]:
        parts = [
            f"at={value_or_dash(attempt.get('attempted_at'))}",
            f"result={value_or_dash(attempt.get('result'))}",
            f"chat_id={value_or_dash(attempt.get('chat_id'))}",
            f"signal_type={value_or_dash(attempt.get('signal_type'))}",
            f"event_id={value_or_dash(attempt.get('event_id'))}",
            f"ttl_ms={value_or_dash(attempt.get('ttl_ms'))}",
            f"bubble_policy={value_or_dash(attempt.get('bubble_policy'))}",
            f"queued={value_or_dash(attempt.get('queued'))}",
        ]
        if attempt.get("skip_reason"):
            parts.append(f"skip_reason={attempt.get('skip_reason')}")
        print(f"    - {' '.join(parts)}")


def print_queue_summary(snapshot: JsonObject, now: datetime) -> None:
    events = runtime_queue(snapshot)
    source_count = sum(1 for event in events if event.get("source") == "chat_reactions")
    ages = [age for event in events if (age := event_age_ms(event, now)) is not None]
    print("runtime_queue summary")
    print(f"  total: {len(events)}")
    print(f"  source == chat_reactions: {source_count}")
    if ages:
        print(
            "  age: "
            f"newest={format_ms(min(ages))} oldest={format_ms(max(ages))} "
            f"average={format_ms(sum(ages) / len(ages))}"
        )
    else:
        print("  age: unknown")


def print_latest_summary(
    snapshot: JsonObject,
    attempt: JsonObject | None,
    latest_any: JsonObject | None,
    chat_id: str | None,
    now: datetime,
) -> list[str]:
    failures = []
    if attempt is None:
        if latest_any is None:
            print("Latest emitted attempt: none")
            failures.append("no emitted attempts found")
        else:
            print("Latest emitted attempt: none for requested chat id")
            print(
                "  latest other chat: "
                f"chat_id={value_or_dash(latest_any.get('chat_id'))} "
                f"event_id={value_or_dash(latest_any.get('event_id'))} "
                f"signal_type={value_or_dash(latest_any.get('signal_type'))}"
            )
            failures.append("wrong chat id: latest emitted attempt is for a different chat")
        if chat_id:
            print(f"Expected UI: BuddyChatCompanion(chatId={chat_id}) has no emitted reaction to show")
        return failures

    queued_event = find_runtime_queue_event(snapshot, attempt.get("event_id"))
    now_playing_event = find_now_playing_event(snapshot, attempt.get("event_id"))
    event = queued_event or now_playing_event
    queued = queued_event is not None
    age = event_age_ms(event, now) if event else attempt_age_ms(attempt, now)
    remaining = ttl_remaining_ms(event, attempt, now)
    print("Latest emitted attempt")
    print(f"  event_id: {value_or_dash(attempt.get('event_id'))}")
    print(f"  chat_id: {value_or_dash(attempt.get('chat_id'))}")
    print(f"  signal_type: {value_or_dash(attempt.get('signal_type'))}")
    print(f"  ttl_ms: {value_or_dash(attempt.get('ttl_ms'))}")
    print(f"  bubble_policy: {value_or_dash(attempt.get('bubble_policy'))}")
    print(f"  present_in_runtime_queue: {value_or_dash(queued)}")
    print(f"  present_as_now_playing: {value_or_dash(now_playing_event is not None)}")
    print(f"  age: {format_ms(age)}")
    print(f"  ttl_remaining: {format_ms(remaining)}")
    if event:
        print(f"  runtime_source: {value_or_dash(event.get('source'))}")
        print(f"  runtime_status: {value_or_dash(event.get('status'))}")
        print(f"  runtime_priority: {value_or_dash(event.get('priority'))}")
        if chat_id and event.get("chat_id") != chat_id:
            failures.append("wrong chat id: queued event chat_id differs from requested chat id")
    if not queued and is_within_ttl(event, attempt, now):
        failures.append("event missing from runtime_queue while still within TTL")
    expected_chat_id = attempt.get("chat_id") if isinstance(attempt.get("chat_id"), str) else chat_id
    print(
        f"Expected UI: BuddyChatCompanion(chatId={value_or_dash(expected_chat_id)}) "
        f"should show speech_text={safe_runtime_text(event)}"
    )
    return failures


def collect_snapshots(port: int, watch: float) -> list[JsonObject]:
    deadline = time.monotonic() + max(0.0, watch)
    snapshots = []
    while True:
        snapshots.append(fetch_snapshot(port))
        if watch <= 0 or time.monotonic() >= deadline:
            return snapshots
        time.sleep(min(1.0, max(0.0, deadline - time.monotonic())))


def recent_emitted_in_window(
    snapshots: list[JsonObject],
    chat_id: str | None,
    started_at: datetime,
) -> bool:
    for snapshot in snapshots:
        for attempt in emitted_attempts(snapshot):
            if chat_id and attempt.get("chat_id") != chat_id:
                continue
            attempted_at = parse_timestamp(attempt.get("attempted_at"))
            if attempted_at is None or attempted_at >= started_at:
                return True
    return False


def disabled_failures(snapshot: JsonObject) -> list[str]:
    failures = []
    settings = as_dict(snapshot.get("settings"))
    if snapshot.get("enabled") is not True:
        failures.append("Buddy is disabled or service is not initialized")
    if settings.get("enabled") is not True:
        failures.append("settings.enabled is not true")
    if settings.get("chat_reactions_enabled") is not True:
        failures.append("settings.chat_reactions_enabled is not true")
    if settings.get("message_observation_enabled") is not True:
        failures.append("settings.message_observation_enabled is not true")
    return failures


def run() -> int:
    args = parse_args()
    started_at = datetime.now(timezone.utc)
    try:
        snapshots = collect_snapshots(args.port, args.watch)
    except urllib.error.URLError as error:
        print(f"FAIL: could not reach /v1/buddy on port {args.port}: {error}", file=sys.stderr)
        return 2
    except (json.JSONDecodeError, RuntimeError) as error:
        print(f"FAIL: invalid /v1/buddy response: {error}", file=sys.stderr)
        return 2

    snapshot = snapshots[-1]
    now = datetime.now(timezone.utc)
    print_settings_summary(snapshot)
    print()
    print_recent_attempts(snapshot)
    print()
    print_queue_summary(snapshot, now)
    print()
    attempt, latest_any = latest_emitted(snapshot, args.chat_id)
    failures = disabled_failures(snapshot)
    failures.extend(print_latest_summary(snapshot, attempt, latest_any, args.chat_id, now))
    if args.expect_visible and args.watch > 0 and not recent_emitted_in_window(
        snapshots,
        args.chat_id,
        started_at,
    ):
        failures.append("no recent emitted attempt found in watch window")

    if failures:
        print()
        print("Verification findings")
        for failure in failures:
            print(f"  FAIL: {failure}")
        return 1 if args.expect_visible else 0

    print()
    print("Verification findings")
    print("  PASS: latest emitted Buddy chat reaction is queued or no visibility expectation was requested")
    return 0


if __name__ == "__main__":
    raise SystemExit(run())

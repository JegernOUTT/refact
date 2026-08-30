#!/usr/bin/env python3
"""Strip enforcement-inert privacy records from trajectory JSON files.

A FileRecord is inert when zone == "normal" and attribution != "declared":
the gate resolves a record by its stored zone string (never re-deriving it
from the path), so such a record can never produce a refusal. Guarded,
blocked, effective:* and declared records are always preserved.

Usage:
    strip_inert_privacy_records.py --dry-run <path>...
    strip_inert_privacy_records.py --apply   <path>...
    strip_inert_privacy_records.py --apply --min-size-mb 10 <dir>...
"""

import argparse
import json
import os
import random
import string
import sys

KEEP_ATTRIBUTIONS = {"declared"}


def is_inert(record):
    if not isinstance(record, dict):
        return False
    return record.get("zone") == "normal" and record.get("attribution") not in KEEP_ATTRIBUTIONS


def strip_document(doc):
    removed = kept = 0
    for message in doc.get("messages") or []:
        if not isinstance(message, dict):
            continue
        privacy = message.get("privacy")
        if not isinstance(privacy, dict):
            continue
        files = privacy.get("files")
        if not isinstance(files, list):
            continue
        survivors = [f for f in files if not is_inert(f)]
        removed += len(files) - len(survivors)
        kept += len(survivors)
        if survivors:
            privacy["files"] = survivors
        else:
            message.pop("privacy", None)
    return removed, kept


def detect_indent(raw):
    newline = raw.find("\n")
    if newline == -1:
        return None
    following = raw[newline + 1 : newline + 9]
    spaces = len(following) - len(following.lstrip(" "))
    return spaces or None


def write_atomic(path, text):
    suffix = "".join(random.choices(string.ascii_lowercase + string.digits, k=8))
    tmp = f"{path}.tmp.{suffix}"
    with open(tmp, "w", encoding="utf-8") as handle:
        handle.write(text)
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(tmp, path)


def collect(paths, min_bytes):
    found = []
    for entry in paths:
        if os.path.isfile(entry):
            found.append(entry)
            continue
        for root, _dirs, names in os.walk(entry):
            for name in names:
                if not name.endswith(".json") or name == "index.json":
                    continue
                full = os.path.join(root, name)
                try:
                    if os.path.getsize(full) >= min_bytes:
                        found.append(full)
                except OSError:
                    pass
    return sorted(set(found))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="+", help="Trajectory JSON files or directories")
    parser.add_argument("--min-size-mb", type=float, default=0.0)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    if args.apply == args.dry_run:
        parser.error("pass exactly one of --apply or --dry-run")

    targets = collect(args.paths, int(args.min_size_mb * 1024 * 1024))
    if not targets:
        print("no matching trajectory files")
        return 0

    total_before = total_after = total_removed = 0
    changed = 0
    for path in targets:
        try:
            raw = open(path, encoding="utf-8").read()
            doc = json.loads(raw)
        except (OSError, ValueError) as error:
            print(f"  SKIP  {path}: {error}")
            continue
        if not isinstance(doc, dict) or "messages" not in doc:
            continue

        before = len(raw)
        removed, kept = strip_document(doc)
        if removed == 0:
            continue

        text = json.dumps(doc, indent=detect_indent(raw), ensure_ascii=False)
        after = len(text)
        total_before += before
        total_after += after
        total_removed += removed
        changed += 1

        print(
            f"  {'rewrote' if args.apply else 'would strip'}  "
            f"{before / 1e6:9.1f}MB -> {after / 1e6:7.2f}MB  "
            f"(-{removed:,} inert, {kept:,} kept)  {path}"
        )
        if args.apply:
            write_atomic(path, text)

    print()
    print(f"files changed      : {changed} of {len(targets)}")
    print(f"records removed    : {total_removed:,}")
    print(f"bytes {'reclaimed' if args.apply else 'reclaimable'}   : "
          f"{(total_before - total_after) / 1e6:.1f} MB "
          f"({total_before / 1e6:.1f} MB -> {total_after / 1e6:.1f} MB)")
    if not args.apply:
        print("\n(dry run - rerun with --apply to write)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

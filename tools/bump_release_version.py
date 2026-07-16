#!/usr/bin/env python3
"""Bump Refact release versions across package manifests."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SEMVER_RE = re.compile(r"^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$")


class VersionBumpError(RuntimeError):
    pass


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def write(path: Path, content: str) -> None:
    path.write_text(content, encoding="utf-8")


def replace_once(path: Path, pattern: str, replacement: str) -> None:
    content = read(path)
    updated, count = re.subn(pattern, replacement, content, count=1, flags=re.MULTILINE)
    if count != 1:
        raise VersionBumpError(f"Expected one replacement in {path}, got {count}")
    write(path, updated)


def replace_json_version_fields(path: Path, package_name: str, version: str, count: int) -> None:
    content = read(path)
    data = json.loads(content)
    if data.get("name") != package_name:
        raise VersionBumpError(f"{path} has unexpected package name {data.get('name')!r}")
    if count == 2:
        root_package = data.get("packages", {}).get("", {})
        if root_package.get("name") != package_name:
            raise VersionBumpError(
                f"{path} has unexpected root lock package name {root_package.get('name')!r}"
            )

    updated, actual_count = re.subn(
        r'(^[ \t]*"version":\s*")[^"]+(",?\s*$)',
        rf"\g<1>{version}\g<2>",
        content,
        count=count,
        flags=re.MULTILINE,
    )
    if actual_count != count:
        raise VersionBumpError(
            f"Expected {count} version replacement(s) in {path}, got {actual_count}"
        )
    json.loads(updated)
    write(path, updated)


def replace_cargo_package_version(path: Path, package_name: str, version: str) -> None:
    content = read(path)
    pattern = (
        r'(\[package\]\s+name\s*=\s*"'
        + re.escape(package_name)
        + r'"\s+version\s*=\s*")[^"]+(" )'
    )
    updated, count = re.subn(pattern, rf"\g<1>{version}\g<2>", content, count=1)
    if count != 1:
        pattern = (
            r'(\[package\]\s+name\s*=\s*"'
            + re.escape(package_name)
            + r'"\s+version\s*=\s*")[^"]+(")'
        )
        updated, count = re.subn(pattern, rf"\g<1>{version}\g<2>", content, count=1)
    if count != 1:
        raise VersionBumpError(f"Expected one Cargo package replacement in {path}, got {count}")
    write(path, updated)


def replace_cargo_lock_version(path: Path, package_name: str, version: str) -> None:
    content = read(path)
    pattern = (
        r'(\[\[package\]\]\s+name\s*=\s*"'
        + re.escape(package_name)
        + r'"\s+version\s*=\s*")[^"]+(")'
    )
    updated, count = re.subn(pattern, rf"\g<1>{version}\g<2>", content, count=1)
    if count != 1:
        raise VersionBumpError(f"Expected one Cargo.lock replacement in {path}, got {count}")
    write(path, updated)


def bump(version: str) -> list[Path]:
    if not SEMVER_RE.fullmatch(version):
        raise VersionBumpError(f"Invalid SemVer version: {version}")

    files = [
        ROOT / "plugins/intellij/gradle.properties",
        ROOT / "plugins/vscode/package.json",
        ROOT / "plugins/vscode/package-lock.json",
        ROOT / "refact-agent/gui/package.json",
        ROOT / "refact-agent/gui/package-lock.json",
        ROOT / "refact-agent/engine/Cargo.toml",
        ROOT / "refact-agent/engine/Cargo.lock",
    ]

    replace_once(
        files[0],
        r"^(pluginVersion\s*=\s*).+$",
        rf"\g<1>{version}",
    )
    replace_json_version_fields(files[1], "codify", version, count=1)
    replace_json_version_fields(files[2], "codify", version, count=2)
    replace_json_version_fields(files[3], "refact-chat-js", version, count=1)
    replace_json_version_fields(files[4], "refact-chat-js", version, count=2)
    replace_cargo_package_version(files[5], "refact-lsp", version)
    replace_cargo_lock_version(files[6], "refact-lsp", version)

    return files


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", help="New release version, for example 8.0.4")
    args = parser.parse_args()

    changed = bump(args.version)
    print(f"Updated release version to {args.version} in:")
    for path in changed:
        print(f"- {path.relative_to(ROOT)}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Opt-in API-only Rust development builds; never installs toolchains or edits config."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

NIGHTLY = "nightly-2026-11-04"
ENGINE = Path(__file__).resolve().parents[2] / "refact-agent" / "engine"
MIN_MEMORY = 48 * 1024**3


def positive(value: str) -> int:
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def arguments(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("check", "build", "experimental-lib"), default="check")
    parser.add_argument("--package", default="refact-lsp", help="workspace package (check/library only)")
    parser.add_argument("--toolchain", choices=("stable", NIGHTLY), help="default: ambient cargo toolchain")
    parser.add_argument("--jobs", type=positive, default=4, help="Cargo jobs; experimental frontend stays at 4")
    parser.add_argument("--cranelift", action="store_true", help="experimental root library only; component must already be installed")
    parser.add_argument("--allow-low-memory", action="store_true", help="override experimental 48 GiB available RAM guard")
    parser.add_argument("--allow-wrapper", action="store_true", help="preserve explicitly configured compiler wrappers despite incremental incompatibility")
    parser.add_argument("--offline", dest="offline", action="store_true", default=True)
    parser.add_argument("--online", dest="offline", action="store_false", help="allow Cargo dependency downloads (never toolchain installs)")
    parser.add_argument("--dry-run", action="store_true", help="print JSON only; no subprocesses or writes")
    parser.add_argument("extra", nargs=argparse.REMAINDER, help="after --: --features VALUE, --no-default-features, --verbose, --quiet")
    args = parser.parse_args(argv)
    if not re.fullmatch(r"[A-Za-z0-9_-]+", args.package):
        parser.error("--package must be a workspace package name")
    experimental = args.mode == "experimental-lib"
    if args.cranelift and not experimental:
        parser.error("--cranelift requires --mode experimental-lib")
    if not experimental and args.toolchain == NIGHTLY:
        parser.error("nightly requires --mode experimental-lib")
    if experimental and args.toolchain == "stable":
        parser.error("experimental-lib requires the pinned nightly")
    if args.mode == "build" and args.package != "refact-lsp":
        parser.error("--package override is only supported for check/library modes")
    extra = args.extra[1:] if args.extra[:1] == ["--"] else args.extra
    index = 0
    while index < len(extra):
        flag = extra[index]
        if flag == "--features":
            index += 1
            if index == len(extra) or extra[index].startswith("-"):
                parser.error("--features requires a value")
        elif flag not in ("--no-default-features", "--verbose", "--quiet"):
            parser.error(f"unsupported extra Cargo argument: {flag}")
        index += 1
    args.extra = extra
    return args


def available_memory():
    try:
        fields = dict(line.split(":", 1) for line in Path("/proc/meminfo").read_text().splitlines())
        available = int(fields["MemAvailable"].split()[0]) * 1024
        memberships = [line[3:] for line in Path("/proc/self/cgroup").read_text().splitlines()
                       if line.startswith("0::")]
        if len(memberships) != 1:
            return None
        member = Path(memberships[0])
        if not member.is_absolute() or ".." in member.parts or "\\" in str(member):
            return None
        mounts = []
        for line in Path("/proc/self/mountinfo").read_text().splitlines():
            mount, separator, filesystem = line.partition(" - ")
            parts = mount.split()
            if separator and filesystem.split()[0] == "cgroup2":
                # A subtree mount hides effective ancestor limits: refuse to guess.
                if len(parts) < 5 or parts[3] != "/" or "\\" in parts[4]:
                    return None
                mounts.append(Path(parts[4]))
        if len(mounts) != 1 or not mounts[0].is_absolute():
            return None
        root = mounts[0]
        current = root / member.relative_to("/")
        if root.resolve() != root or current.resolve() != current:
            return None
        while True:
            limit = (current / "memory.max").read_text().strip()
            if limit != "max":
                maximum = int(limit)
                used = int((current / "memory.current").read_text())
                if maximum < 0 or used < 0:
                    return None
                available = min(available, max(0, maximum - used))
            if current == root:
                return available if available >= 0 else None
            current = current.parent
    except (OSError, KeyError, ValueError, IndexError, RuntimeError):
        return None


def plan(args, environ):
    experimental = args.mode == "experimental-lib"
    toolchain = NIGHTLY if experimental else args.toolchain
    target = ENGINE / "target"
    if experimental:
        target /= "rust-dev/nightlyCranelift" if args.cranelift else "rust-dev/nightlyLLVM"
    target = target.resolve()
    if not target.is_relative_to(ENGINE.resolve()):
        raise ValueError("target directory resolves outside this checkout's engine")
    command = ["cargo"] + ([f"+{toolchain}"] if toolchain else [])
    command += ["rustc" if experimental else args.mode, "-p", args.package]
    command += ["--bin", "refact-lsp"] if args.mode == "build" else ["--lib"]
    command += ["--locked", "--jobs", str(args.jobs), "--target-dir", str(target)]
    if args.offline:
        command.append("--offline")
    command += args.extra
    if experimental:
        command += ["--", "-Zunstable-options", "--jobs-frontend=4"]
        if args.cranelift:
            command.append("-Zcodegen-backend=cranelift")
    updates = {"CARGO_INCREMENTAL": "1", "REFACT_SKIP_GUI_BUILD": "1", "RUSTUP_AUTO_INSTALL": "0"}
    warnings = ["API-only: GUI build skipped; embedded assets are unvalidated. Not a distribution build."]
    errors = []
    if environ.get("RUSTC_BOOTSTRAP"):
        errors.append("RUSTC_BOOTSTRAP is configured; run without it (helper never enables bootstrap)")
    wrappers = [key for key in ("RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
                                "CARGO_BUILD_RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER")
                if environ.get(key)]
    if wrappers:
        warnings.append("Compiler wrappers preserved; incremental/sccache may be incompatible: " + ", ".join(wrappers))
        if not args.allow_wrapper:
            errors.append("configured compiler wrapper requires explicit --allow-wrapper")
    if experimental:
        memory = available_memory()
        if memory is None or memory < MIN_MEMORY:
            warnings.append("Experimental builds measured ~42 GiB RAM; 48 GiB available required; available=" + ("unknown" if memory is None else f"{memory / 1024**3:.1f} GiB"))
            if not args.allow_low_memory:
                errors.append("insufficient/unknown available RAM; free memory or explicitly use --allow-low-memory")
    return command, updates, warnings, errors, toolchain


def main(argv=None):
    args = arguments(argv)
    command, updates, warnings, errors, toolchain = plan(args, os.environ)
    if args.dry_run:
        print(json.dumps({"cwd": str(ENGINE), "command": command, "env_overrides": updates,
                          "warnings": warnings, "execution_blockers": errors}, indent=2))
        return 0
    for warning in warnings:
        print("warning: " + warning, file=sys.stderr)
    if errors:
        raise ValueError("; ".join(errors))
    env = dict(os.environ, **updates)
    probe = ["rustup", "which"] + (["--toolchain", toolchain] if toolchain else []) + ["cargo"]
    result = subprocess.run(probe, cwd=ENGINE, env=env, capture_output=True, text=True)
    if result.returncode:
        raise ValueError("Cargo/toolchain is not installed. Install explicitly with rustup before retrying: " + (toolchain or "your selected toolchain"))
    try:
        import fcntl
    except ImportError:
        raise ValueError("checkout locking requires Unix fcntl; use Cargo directly on this platform") from None
    lock_dir = ENGINE / "target"
    lock_dir.mkdir(exist_ok=True)
    with (lock_dir / ".rust-dev.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ValueError("another rust-dev helper is running in this checkout") from None
        return subprocess.run(command, cwd=ENGINE, env=env).returncode


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError) as error:
        print(f"rust-dev: {error}", file=sys.stderr)
        sys.exit(1)

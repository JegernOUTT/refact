#!/usr/bin/env bash
set -euo pipefail

exec "${SETUP_CACHE_PYTHON:-python3}" - "$@" <<'PY'
import argparse
import copy
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import stat
import sys
import tempfile

try:
    import tomllib
except ImportError:
    try:
        import tomli as tomllib
    except ImportError:
        sys.exit("setup-cache: full TOML parser unavailable; use SETUP_CACHE_PYTHON=/path/to/already-installed/python3.11 (or Python with tomli). No files changed; no parser is installed automatically")


def config_at(directory):
    legacy = directory / "config"
    modern = directory / "config.toml"
    if legacy.is_symlink() or modern.is_symlink():
        raise ValueError(f"Cargo config symlink requires manual inspection: {directory}")
    if legacy.exists() and modern.exists():
        print(f"Both configs exist; Cargo selects {legacy}", file=sys.stderr)
    return legacy if legacy.exists() else modern


def read_config(path):
    if path.is_symlink():
        raise ValueError(f"Config symlink refused: {path}")
    if path.exists() and not path.is_file():
        raise ValueError(f"Config is not a regular file: {path}")
    return path.read_text() if path.exists() else ""


def build_values(text):
    parsed = tomllib.loads(text)
    if "include" in parsed:
        raise ValueError("Cargo include directives require manual inspection; nothing changed")
    build = parsed.get("build", {})
    if not isinstance(build, dict):
        raise ValueError("Cargo build must be a table")
    return parsed, build


def add_settings(text, settings):
    parsed, _ = build_values(text)
    expected = copy.deepcopy(parsed)
    expected.setdefault("build", {}).update(settings)
    lines = text.splitlines(keepends=True)
    assignments = "".join(f"{key} = {json.dumps(value)}\n" for key, value in settings.items())
    candidates = ["".join(f"build.{key} = {json.dumps(value)}\n" for key, value in settings.items()) + text]
    for index, line in enumerate(lines):
        if re.fullmatch(r"\s*\[\s*build\s*\]\s*(?:#.*)?\n?", line):
            candidates.append("".join(lines[:index + 1]).rstrip("\n") + "\n" + assignments + "".join(lines[index + 1:]))
    candidates.append(text.rstrip("\n") + "\n\n[build]\n" + assignments)
    for candidate in candidates:
        try:
            if tomllib.loads(candidate) == expected:
                return candidate
        except tomllib.TOMLDecodeError:
            pass
    raise ValueError("Cannot safely extend this TOML layout; add the displayed build settings manually")


def atomic_config(path, original, updated):
    if path.absolute() != path.resolve():
        raise ValueError(f"Symlinked config destination refused: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    mode = 0o600
    if path.exists():
        info = path.lstat()
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_nlink != 1:
            raise ValueError(f"Config is not a caller-owned regular file: {path}")
        mode = stat.S_IMODE(info.st_mode)
        fd, backup = tempfile.mkstemp(prefix=path.name + ".bak.", dir=path.parent)
        with os.fdopen(fd, "w") as stream:
            stream.write(original)
            stream.flush()
            os.fsync(stream.fileno())
        print(f"Backup: {backup}")
    fd, temporary = tempfile.mkstemp(prefix=path.name + ".tmp.", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            stream.write(updated)
            stream.flush()
            os.fchmod(stream.fileno(), mode)
            os.fsync(stream.fileno())
        if read_config(path) != original:
            raise ValueError("Config changed during setup; retry after other writers stop")
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def main():
    parser = argparse.ArgumentParser(description="Inspect Cargo cache configuration without contacting sccache. Default: dry run.")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--status", action="store_true")
    mode.add_argument("--dry-run", action="store_true")
    mode.add_argument("--apply", action="store_true")
    parser.add_argument("--cwd", type=Path, default=Path.cwd(), help="Cargo invocation directory (no manifest-path/CLI config overrides)")
    parser.add_argument("--cargo-home", type=Path, default=Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))))
    args = parser.parse_args()
    cwd = args.cwd.resolve(strict=True)
    home = args.cargo_home.absolute()
    if home != home.resolve():
        raise ValueError("Symlinked Cargo home requires manual inspection")
    target = config_at(home)
    paths = [target]
    for ancestor in [*reversed(cwd.parents), cwd]:
        local = config_at(ancestor / ".cargo")
        if local not in paths:
            paths.append(local)
    effective = {}
    env_settings = {}
    for path in paths:
        text = read_config(path)
        parsed, build = build_values(text)
        effective.update(build)
        env_settings.update(parsed.get("env", {}))
        if text:
            print(f"Active config: {path}")
    wrapper = os.environ.get("RUSTC_WRAPPER", os.environ.get("CARGO_BUILD_RUSTC_WRAPPER", effective.get("rustc-wrapper", "")))
    incremental = os.environ.get("CARGO_INCREMENTAL", os.environ.get("CARGO_BUILD_INCREMENTAL", effective.get("incremental", "unset")))
    for key in ("RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_INCREMENTAL", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR"):
        if key in env_settings:
            raise ValueError(f"Cargo [env].{key} requires manual precedence inspection")
    configured_env = env_settings.get("CARGO_INCREMENTAL")
    if configured_env is not None:
        value = configured_env.get("value") if isinstance(configured_env, dict) else configured_env
        force = isinstance(configured_env, dict) and configured_env.get("force", False)
        if force or "CARGO_INCREMENTAL" not in os.environ:
            incremental = value
    print(f"Effective rustc-wrapper: {wrapper or 'unset (not enabled)'}")
    print(f"Effective incremental: {incremental}")
    workspace_wrapper = os.environ.get("RUSTC_WORKSPACE_WRAPPER", os.environ.get("CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER", effective.get("rustc-workspace-wrapper", "")))
    print(f"Workspace wrapper (preserved): {workspace_wrapper or 'unset'}")
    print(f"sccache executable on PATH: {shutil.which('sccache') or 'not found; not installed by this script'}")
    print("Configuration only: no daemon, cache-hit, runtime or performance verification.")
    print("Keep mutable target outputs per checkout; shared compiler/download caches are not universal build artifacts.")
    if args.status:
        return
    if os.environ.get("CARGO_TARGET_DIR") or os.environ.get("CARGO_BUILD_TARGET_DIR") or effective.get("target-dir"):
        raise ValueError("Explicit target-dir detected; verify it is checkout-owned before enabling cache setup")
    if configured_env is not None and str(incremental).lower() not in ("0", "false"):
        raise ValueError("Conflicting [env].CARGO_INCREMENTAL preserved; resolve it manually")
    if "incremental" in effective and effective["incremental"] is not False:
        raise ValueError("Effective build.incremental conflicts; resolve it manually")
    if str(incremental).lower() not in ("unset", "0", "false"):
        raise ValueError("Current incremental environment conflicts; invoke with CARGO_INCREMENTAL=0 explicitly")
    original = read_config(target)
    _, local_build = build_values(original)
    for value in (wrapper, effective.get("rustc-wrapper", ""), local_build.get("rustc-wrapper", "")):
        if value and Path(value).name not in ("sccache", "sccache.exe"):
            raise ValueError(f"Custom rustc-wrapper preserved: {value!r}; configure wrapper composition manually")
    settings = {}
    if "rustc-wrapper" not in local_build:
        settings["rustc-wrapper"] = "sccache"
    elif not local_build["rustc-wrapper"]:
        raise ValueError("Explicit empty rustc-wrapper preserved; remove it manually to opt in")
    if "incremental" not in local_build:
        settings["incremental"] = False
    elif local_build["incremental"] is not False:
        raise ValueError("Explicit build.incremental preserved; set it to false manually to opt in")
    updated = add_settings(original, settings) if settings else original
    print(f"{'Apply' if args.apply else 'Dry run'} target: {target}")
    print("Requested build settings: rustc-wrapper = sccache; incremental = false")
    if args.apply and updated != original:
        atomic_config(target, original, updated)
        print("Applied user Cargo config; higher-precedence config/env can still override it.")
    elif updated == original:
        print("No config changes needed.")
    else:
        print("No files changed; use --apply to write a backup and atomically update config.")
    if str(incremental).lower() not in ("0", "false"):
        print("Current invocation does not independently establish incremental=off; use the explicit environment below.")
    print("For each shell/product child invocation (not .bashrc):")
    print("env CARGO_HOME=" + shlex.quote(str(home)) + " CARGO_INCREMENTAL=0 RUSTC_WRAPPER=sccache SCCACHE_CACHE_SIZE=" + shlex.quote(os.environ.get("SCCACHE_CACHE_SIZE", "50G")) + " cargo <arguments>")
    print("[env].CARGO_INCREMENTAL force=true can override that recommendation; resolve any conflict manually.")
    print("Refact scrubbed exec needs explicit request env/passthrough; this script changes no engine policy or parent environment.")


try:
    main()
except (OSError, ValueError, TypeError) as error:
    sys.exit(f"setup-cache: {error}; no automatic conflict replacement")
PY

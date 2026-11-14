#!/usr/bin/env python3
"""Strict producer for the schema-1 runtime manifest.

The wire format mirrors refact-agent/engine/crates/refact-self-update/src/runtime_manifest.rs
(serde field order, deny_unknown_fields, canonical artifact ordering).  Everything recorded here
is derived from real files: build inputs from committed lock/config bytes, artifact identity from
the produced archives, component versions from the manifests packed inside those archives.

Subcommands:
  build-inputs  print {"build_inputs": {...}, "build_inputs_digest": "<hex>"} (shared with Tests)
  produce       write the canonical manifest for a release directory
  validate      validate an existing manifest file against the schema rules
  verify-qualification  fail closed unless a Tests report qualifies this exact SHA/build inputs
  verify-bundle         check a downloaded release bundle against its manifest and list assets
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import re
import subprocess
import sys
import tarfile
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

SCHEMA_VERSION = 1
MAX_MANIFEST_BYTES = 256 * 1024
MAX_ARTIFACTS = 64
MAX_EXTRACTED_FILES = 256
MAX_ARCHIVE_MEMBER_BYTES = 1 << 31

JAVASCRIPT_LOCKS = (
    "plugins/vscode/package-lock.json",
    "refact-agent/engine/crates/refact-browser/injected/package-lock.json",
    "refact-agent/gui/package-lock.json",
    "refact-agent/vite-plugin-design/package-lock.json",
)
# Build recipes that shape shipped bytes. Every workflow file is added dynamically (sorted) so the
# Tests and Build All recipes present in the same checkout are both bound.
CONFIG_FILES = (
    ".cargo/config.toml",
    "packaging/runtime-compatibility.json",
    "plugins/intellij/build.gradle.kts",
    "plugins/intellij/gradle.properties",
    "plugins/intellij/gradle/libs.versions.toml",
    "plugins/intellij/gradle/wrapper/gradle-wrapper.properties",
    "plugins/intellij/settings.gradle.kts",
    "plugins/vscode/.vscodeignore",
    "plugins/vscode/package.json",
    "plugins/vscode/tsconfig.json",
    "refact-agent/engine/.cargo/config.toml",
    "refact-agent/engine/Cargo.toml",
    "refact-agent/engine/Cross.toml",
    "refact-agent/engine/build.rs",
    "refact-agent/engine/build_support/gui_artifact.rs",
    "refact-agent/engine/build_support/gui_artifact/trusted_controller.rs",
    "refact-agent/gui/package.json",
    "refact-agent/gui/tsconfig.json",
    "refact-agent/gui/tsconfig.node.json",
    "refact-agent/gui/vite.config.ts",
    "refact-agent/gui/vite.node.config.ts",
    "rust-toolchain.toml",
    "tools/dev/compute_build_version.sh",
    "tools/dev/package_engine_release_assets.sh",
    "tools/dev/prepare_ci_gui.sh",
    "tools/dev/runtime_manifest.py",
)
COMPATIBILITY_FILE = "packaging/runtime-compatibility.json"
NATIVE_TARGETS = (
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "i686-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
)
# Targets Build All actually produces (x86_64-apple-darwin is intentionally not built).
DEFAULT_RUNTIME_TARGETS = (
    "x86_64-pc-windows-msvc",
    "i686-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "aarch64-apple-darwin",
)
CHANNELS = ("stable", "preview", "nightly")
JETBRAINS_PLUGIN_ID = "com.smallcloud.codify"

RUST_TOOLCHAIN_RE = re.compile(r"^(1\.[0-9]+\.[0-9]+|nightly-[0-9]{4}-[0-9]{2}-[0-9]{2})$")
NODE_VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")
PROFILE_RE = re.compile(r"^[a-z][a-z0-9-]{0,63}$")
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")


class ManifestError(Exception):
    pass


def require(ok: bool, message: str) -> None:
    if not ok:
        raise ManifestError(message)


# ---------------------------------------------------------------- primitives (mirror Rust helpers)
def _text(s, limit: int) -> bool:
    return isinstance(s, str) and 0 < len(s.encode()) <= limit and not any(
        ord(c) < 0x20 or 0x7F <= ord(c) <= 0x9F for c in s
    )


def _digest(s, length: int) -> bool:
    return isinstance(s, str) and len(s) == length and all(c in "0123456789abcdef" for c in s)


def safe_path(s) -> bool:
    return (
        _text(s, 512)
        and not s.startswith("/")
        and "\\" not in s
        and ":" not in s
        and all(p not in ("", ".", "..") for p in s.split("/"))
    )


def portable_segment(s: str) -> bool:
    base = s.split(".")[0].upper()
    return (
        bool(s)
        and s not in (".", "..")
        and not s.endswith((".", " "))
        and all(c.isascii() and (c.isalnum() or c in "._-+") for c in s)
        and base not in ("CON", "PRN", "AUX", "NUL")
        and not (len(base) == 4 and base[:3] in ("COM", "LPT") and base[3] in "123456789")
    )


def semver(s) -> bool:
    if not _text(s, 128):
        return False
    core, _, suffix = s.partition("-")
    parts = core.split(".")
    if len(parts) != 3 or not all(p.isascii() and p.isdigit() and (len(p) == 1 or p[0] != "0") for p in parts):
        return False
    if "-" in s:
        return all(p and p.isascii() and p.isalnum() for p in re.split(r"[.-]", suffix))
    return True


def _uint(v) -> bool:
    return isinstance(v, int) and not isinstance(v, bool) and 0 <= v < 1 << 64


def compact(value) -> bytes:
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


# ---------------------------------------------------------------- build inputs (shared with Tests)
def path_map_digest(entries: dict) -> str:
    """Rust BuildInputs::javascript_locks_digest: sha256 of compact sorted path->sha256 JSON map."""
    require(isinstance(entries, dict) and 0 < len(entries) <= 32, "invalid JavaScript lockfile inventory")
    require(all(safe_path(k) and _digest(v, 64) for k, v in entries.items()), "invalid inventory entry")
    return sha256_bytes(compact(dict(sorted(entries.items()))))


def config_digest(entries: dict) -> str:
    # Same canonical map scheme; not bounded to 32 entries because workflows are included.
    require(entries and all(safe_path(k) and _digest(v, 64) for k, v in entries.items()), "invalid config inventory")
    return sha256_bytes(compact(dict(sorted(entries.items()))))


def _hash_inventory(root: Path, paths) -> dict:
    out = {}
    for rel in paths:
        require(safe_path(rel), f"unsafe inventory path: {rel!r}")
        path = root / rel
        require(path.is_file() and not path.is_symlink(), f"missing build input file: {rel}")
        out[rel] = sha256_file(path)
    return out


def config_paths(root: Path) -> list:
    workflows = sorted(
        p.relative_to(root).as_posix()
        for p in (root / ".github" / "workflows").glob("*")
        if p.is_file() and p.suffix in (".yml", ".yaml")
    )
    require(workflows, "no workflow recipes found")
    return sorted(set(CONFIG_FILES) | set(workflows))


def compute_build_inputs(root, rust_toolchain: str, node_version: str, profile: str) -> dict:
    root = Path(root)
    require(isinstance(rust_toolchain, str) and bool(RUST_TOOLCHAIN_RE.match(rust_toolchain)),
            f"rust_toolchain must be an exact x.y.z or nightly-YYYY-MM-DD, got {rust_toolchain!r}")
    require(isinstance(node_version, str) and bool(NODE_VERSION_RE.match(node_version)),
            f"node_version must be an exact x.y.z, got {node_version!r}")
    require(isinstance(profile, str) and bool(PROFILE_RE.match(profile)), f"invalid profile {profile!r}")
    cargo_lock = root / "refact-agent/engine/Cargo.lock"
    require(cargo_lock.is_file(), "missing build input file: refact-agent/engine/Cargo.lock")
    return {
        "rust_toolchain": rust_toolchain,
        "node_version": node_version,
        "profile": profile,
        "cargo_lock_sha256": sha256_file(cargo_lock),
        "javascript_locks_sha256": path_map_digest(_hash_inventory(root, JAVASCRIPT_LOCKS)),
        "config_sha256": config_digest(_hash_inventory(root, config_paths(root))),
    }


BUILD_INPUT_FIELDS = ("rust_toolchain", "node_version", "profile", "cargo_lock_sha256",
                      "javascript_locks_sha256", "config_sha256")


def validate_build_inputs(bi) -> dict:
    require(isinstance(bi, dict) and set(bi) == set(BUILD_INPUT_FIELDS), "invalid build inputs fields")
    require(all(_text(bi[k], 128) for k in BUILD_INPUT_FIELDS[:3]), "invalid build inputs")
    require(all(_digest(bi[k], 64) for k in BUILD_INPUT_FIELDS[3:]), "invalid build inputs")
    return {k: bi[k] for k in BUILD_INPUT_FIELDS}


def build_inputs_digest(bi: dict) -> str:
    """Rust BuildInputs::digest(): sha256 of compact JSON in struct field order."""
    return sha256_bytes(compact(validate_build_inputs(bi)))


# ---------------------------------------------------------------- manifest schema
def _exact(obj, keys, optional=(), what="object"):
    require(isinstance(obj, dict), f"{what} must be an object")
    unknown = set(obj) - set(keys) - set(optional)
    require(not unknown, f"unknown {what} fields: {sorted(unknown)}")
    require(all(k in obj for k in keys), f"missing {what} fields")


def _fmt_ok(component, target, fmt):
    if component == "runtime":
        return fmt == ("zip" if "windows" in target else "tar_gz")
    return {"vscode": fmt == "vsix", "jetbrains": fmt in ("zip", "jar"), "gui": fmt == "tar_gz"}[component]


def canonical_manifest(m) -> dict:
    """Validate like RuntimeManifest::try_from and return the canonical (serde-ordered) value."""
    require(len(compact(m)) <= MAX_MANIFEST_BYTES, "manifest byte limit exceeded")
    _exact(m, ("schema_version", "repository", "source_sha", "workflow", "run", "build", "build_inputs",
               "component_versions", "compatibility", "artifacts", "tests"), ("qualification_ref",), "manifest")
    require(m["schema_version"] == SCHEMA_VERSION and _uint(m["schema_version"]), "unsupported manifest schema")
    r = m["repository"]
    _exact(r, ("owner", "name", "id"), what="repository")
    require(_uint(r["id"]) and r["id"] > 0 and all(
        _text(s, 100) and all(c.isascii() and (c.isalnum() or c in "-_.") for c in s) and s not in (".", "..")
        for s in (r["owner"], r["name"])), "invalid repository")
    w = m["workflow"]
    _exact(w, ("path", "id"), what="workflow")
    require(_uint(w["id"]) and w["id"] > 0 and safe_path(w["path"]) and w["path"].startswith(".github/workflows/")
            and w["path"].endswith((".yml", ".yaml")), "invalid workflow")
    run = m["run"]
    _exact(run, ("id", "attempt"), what="run")
    require(_uint(run["id"]) and _uint(run["attempt"]) and run["id"] > 0 and run["attempt"] > 0, "invalid run")
    require(_digest(m["source_sha"], 40), "invalid source SHA")
    b = m["build"]
    _exact(b, ("version", "channel"), what="build")
    require(semver(b["version"]), "invalid build version")
    require(b["channel"] in CHANNELS, "invalid channel")
    bi = validate_build_inputs(m["build_inputs"])
    cv = m["component_versions"]
    _exact(cv, ("runtime", "gui", "vscode", "jetbrains"), what="component_versions")
    require(semver(cv["runtime"]) and all(
        _text(v, 128) and all(c.isascii() and (c.isalnum() or c in ".-+_") for c in v)
        for v in (cv["gui"], cv["vscode"], cv["jetbrains"])), "invalid component version")
    comp = m["compatibility"]
    _exact(comp, ("daemon_api", "worker_api", "host_bridge"), what="compatibility")
    compat = {}
    for key in ("daemon_api", "worker_api", "host_bridge"):
        i = comp[key]
        _exact(i, ("min", "max"), what=key)
        require(all(isinstance(i[k], int) and not isinstance(i[k], bool) and 0 <= i[k] < 1 << 32 for k in ("min", "max"))
                and i["min"] <= i["max"], "invalid compatibility interval")
        compat[key] = {"min": i["min"], "max": i["max"]}
    arts = m["artifacts"]
    require(isinstance(arts, list) and 0 < len(arts) <= MAX_ARTIFACTS, "artifact count rejected")
    names, targets, runtime, out_arts = set(), set(), False, []
    for a in arts:
        _exact(a, ("component", "target", "filename", "format", "size", "sha256"), ("extracted_files",), "artifact")
        require(a["component"] in ("runtime", "gui", "vscode", "jetbrains"), "invalid component")
        require(a["format"] in ("tar_gz", "zip", "vsix", "jar"), "invalid format")
        require(isinstance(a["target"], str) and (a["target"] in NATIVE_TARGETS or (
            a["component"] != "runtime" and a["target"] == "universal")), "invalid artifact target")
        require(isinstance(a["filename"], str) and portable_segment(a["filename"]) and "/" not in a["filename"]
                and len(a["filename"]) <= 200 and _uint(a["size"]) and a["size"] > 0 and _digest(a["sha256"], 64),
                "invalid artifact identity")
        lname = a["filename"].lower()
        require(lname not in names and (a["component"], a["target"]) not in targets, "duplicate artifact")
        names.add(lname)
        targets.add((a["component"], a["target"]))
        runtime |= a["component"] == "runtime"
        require(_fmt_ok(a["component"], a["target"], a["format"]), "component/target format mismatch")
        files = a.get("extracted_files", [])
        require(isinstance(files, list) and len(files) <= MAX_EXTRACTED_FILES, "extracted file count rejected")
        fold = "windows" in a["target"] or "apple" in a["target"] or a["target"] == "universal"
        seen, out_files = set(), []
        for f in files:
            _exact(f, ("path", "size", "sha256"), what="extracted file")
            key = f["path"].lower() if isinstance(f["path"], str) and fold else f["path"]
            require(safe_path(f["path"]) and all(portable_segment(p) for p in f["path"].split("/"))
                    and _uint(f["size"]) and f["size"] > 0 and _digest(f["sha256"], 64) and key not in seen,
                    "invalid extracted file")
            seen.add(key)
            out_files.append({"path": f["path"], "size": f["size"], "sha256": f["sha256"]})
        out_files.sort(key=lambda f: f["path"].encode())
        art = {k: a[k] for k in ("component", "target", "filename", "format", "size", "sha256")}
        if out_files:
            art["extracted_files"] = out_files
        out_arts.append(art)
    require(runtime, "runtime bundle required")
    require(m["tests"] in ("passed", "failed", "not_run"), "invalid tests value")
    q = m.get("qualification_ref")
    require(q is None or _text(q, 512), "invalid qualification reference")
    out_arts.sort(key=lambda a: a["filename"].encode())
    out = {
        "schema_version": SCHEMA_VERSION,
        "repository": {"owner": r["owner"], "name": r["name"], "id": r["id"]},
        "source_sha": m["source_sha"],
        "workflow": {"path": w["path"], "id": w["id"]},
        "run": {"id": run["id"], "attempt": run["attempt"]},
        "build": {"version": b["version"], "channel": b["channel"]},
        "build_inputs": bi,
        "component_versions": {k: cv[k] for k in ("runtime", "gui", "vscode", "jetbrains")},
        "compatibility": compat,
        "artifacts": out_arts,
        "tests": m["tests"],
    }
    if q is not None:
        out["qualification_ref"] = q
    # Producer-side channel rule (Rust check_channel): stable requires passed tests; untested must say not_run.
    require(out["tests"] != "failed", "failed tests cannot be published")
    require(out["build"]["channel"] != "stable" or out["tests"] == "passed", "stable manifest requires passed tests")
    require((out["tests"] == "passed") == (q is not None), "qualification_ref must accompany passed tests only")
    return out


def canonical_bytes(m) -> bytes:
    return compact(canonical_manifest(m))


def manifest_digest(m) -> str:
    return sha256_bytes(canonical_bytes(m))


# ---------------------------------------------------------------- artifact inspection
RUNTIME_RE = re.compile(r"^refact-(?P<version>.+)-(?P<target>" + "|".join(map(re.escape, NATIVE_TARGETS)) + r")\.(?P<ext>tar\.gz|zip)$")


def _check_sidecar(path: Path, digest: str) -> None:
    sidecar = path.with_name(path.name + ".sha256")
    require(sidecar.is_file(), f"missing SHA-256 sidecar for {path.name}")
    fields = sidecar.read_text().split()
    require(len(fields) == 2 and fields[0].lower() == digest and fields[1] == path.name,
            f"SHA-256 sidecar mismatch for {path.name}")


BUILD_INFO_HEADER = b"\0REFACT_BUILD_INFO_V1\0"
BUILD_INFO_FOOTER = b"\0END_REFACT_BUILD_INFO\0"
MAX_BUILD_INFO_PAYLOAD = 1024


def _unique_object(pairs):
    keys = [k for k, _ in pairs]
    if len(keys) != len(set(keys)):
        raise ValueError("duplicate key")
    return dict(pairs)


def _reject_constant(name):
    raise ValueError(f"non-finite constant {name}")


def parse_build_info(data: bytes) -> dict:
    records, start = [], 0
    while (i := data.find(BUILD_INFO_HEADER, start)) >= 0:
        start = i + 1
        body = i + len(BUILD_INFO_HEADER)
        j = data.find(BUILD_INFO_FOOTER, body, body + MAX_BUILD_INFO_PAYLOAD + len(BUILD_INFO_FOOTER))
        if j < 0:
            continue
        payload = data[body:j]
        if BUILD_INFO_HEADER in payload:
            continue
        try:
            info = json.loads(payload.decode("utf-8"), object_pairs_hook=_unique_object,
                              parse_constant=_reject_constant)
        except (UnicodeDecodeError, ValueError):
            continue
        if isinstance(info, dict) and set(info) == {"schema_version", "version", "source_sha", "target"}:
            records.append(info)
    require(len(records) == 1, f"runtime binary must contain exactly one build-info record, found {len(records)}")
    info = records[0]
    require(type(info["schema_version"]) is int and info["schema_version"] == 1 and semver(info["version"]) and _digest(info["source_sha"], 40)
            and info["target"] in NATIVE_TARGETS, "invalid runtime build-info record")
    return info


def _runtime_members(path: Path, ext: str, target: str, version: str, source_sha: str) -> list:
    binary = "refact.exe" if "windows" in target else "refact"
    files, payloads = [], []
    if ext == "zip":
        with zipfile.ZipFile(path) as zf:
            for info in zf.infolist():
                require(not info.is_dir() and safe_path(info.filename), f"unsafe archive member in {path.name}")
                require(info.file_size <= MAX_ARCHIVE_MEMBER_BYTES, "archive member too large")
                payloads.append(zf.read(info))
                files.append(info.filename)
    else:
        with tarfile.open(path, "r:gz") as tf:
            for info in tf.getmembers():
                require(info.isfile() and safe_path(info.name), f"unsafe archive member in {path.name}")
                require(info.size <= MAX_ARCHIVE_MEMBER_BYTES, "archive member too large")
                payloads.append(tf.extractfile(info).read())
                files.append(info.name)
    require(files == [binary], f"{path.name} must contain exactly {binary} at its root")
    data = payloads[0]
    info = parse_build_info(data)
    require(info == {"schema_version": 1, "version": version, "source_sha": source_sha, "target": target},
            f"runtime build-info mismatch in {path.name}: {info}")
    return [{"path": binary, "size": len(data), "sha256": sha256_bytes(data)}]


def _gui_version(path: Path) -> str:
    with tarfile.open(path, "r:gz") as tf:
        member = tf.getmember("package/package.json")
        require(member.isfile(), "GUI package.json missing")
        data = json.loads(tf.extractfile(member).read())
    require(data.get("name") == "refact-chat-js", "GUI package is not refact-chat-js")
    return data["version"]


VSIX_PRERELEASE = re.compile(rb'<Property\s+Id="Microsoft\.VisualStudio\.Code\.PreRelease"\s+Value="true"')


def _vscode_version(path: Path, version: str, channel: str) -> str:
    with zipfile.ZipFile(path) as zf:
        data = json.loads(zf.read("extension/package.json"))
        vsixmanifest = zf.read("extension.vsixmanifest")
    require(data.get("publisher") == "smallcloud" and data.get("name") == "codify", "VSIX is not smallcloud.codify")
    actual = data.get("version")
    require(isinstance(actual, str) and re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", actual),
            f"VSIX version must be numeric major.minor.patch: {actual!r}")
    require(actual == version.split("-", 1)[0], f"VSIX version {actual} does not match build {version}")
    require(data.get("refactRuntimeVersion") == version, "VSIX runtime requirement does not match build version")
    require(bool(VSIX_PRERELEASE.search(vsixmanifest)) == (channel != "stable"), "VSIX pre-release flag mismatch")
    return actual


def _jetbrains_version(path: Path) -> str:
    found = []
    with zipfile.ZipFile(path) as zf:
        for name in zf.namelist():
            require(safe_path(name.rstrip("/")), f"unsafe archive member in {path.name}")
            if not (name.endswith(".jar") and "/lib/" in name):
                continue
            with zipfile.ZipFile(io.BytesIO(zf.read(name))) as jar:
                if "META-INF/plugin.xml" not in jar.namelist():
                    continue
                root = ET.fromstring(jar.read("META-INF/plugin.xml"))
                if (root.findtext("id") or "").strip() == JETBRAINS_PLUGIN_ID:
                    found.append((root.findtext("version") or "").strip())
    require(len(found) == 1 and found[0], f"{path.name} must contain exactly one {JETBRAINS_PLUGIN_ID} plugin.xml")
    return found[0]


def collect_artifacts(directory: Path, version: str, runtime_targets, source_sha: str, channel: str) -> tuple:
    directory = Path(directory)
    require(directory.is_dir(), f"artifact directory missing: {directory}")
    entries = sorted(p for p in directory.iterdir())
    artifacts, versions, seen_targets = [], {}, set()
    for p in entries:
        require(p.is_file() and not p.is_symlink(), f"unexpected non-file in artifact directory: {p.name}")
    for p in entries:
        if p.name.endswith(".sha256"):
            require((directory / p.name[: -len(".sha256")]).is_file(), f"orphan sidecar {p.name}")
            continue
        digest, size = sha256_file(p), p.stat().st_size
        require(size > 0, f"empty artifact {p.name}")
        _check_sidecar(p, digest)
        art = {"filename": p.name, "size": size, "sha256": digest}
        m = RUNTIME_RE.match(p.name)
        if m:
            require(m["version"] == version, f"runtime archive version mismatch: {p.name}")
            target = m["target"]
            require(target in runtime_targets, f"unexpected runtime target {target}")
            require((m["ext"] == "zip") == ("windows" in target), f"wrong archive format for {target}")
            art.update(component="runtime", target=target, format="zip" if m["ext"] == "zip" else "tar_gz",
                       extracted_files=_runtime_members(p, m["ext"], target, version, source_sha))
            seen_targets.add(target)
        elif p.name.startswith("refact-chat-js-") and p.name.endswith(".tgz"):
            require("gui" not in versions, "multiple GUI packages")
            versions["gui"] = _gui_version(p)
            art.update(component="gui", target="universal", format="tar_gz")
        elif p.name.endswith(".vsix"):
            require("vscode" not in versions, "multiple VSIX packages")
            versions["vscode"] = _vscode_version(p, version, channel)
            art.update(component="vscode", target="universal", format="vsix")
        elif p.name.endswith(".zip"):
            require("jetbrains" not in versions, "multiple JetBrains plugin ZIPs")
            versions["jetbrains"] = _jetbrains_version(p)
            require(versions["jetbrains"] == version, f"JetBrains plugin version {versions['jetbrains']} != {version}")
            art.update(component="jetbrains", target="universal", format="zip")
        else:
            raise ManifestError(f"unrelated file in release directory: {p.name}")
        artifacts.append(art)
    missing = sorted(set(runtime_targets) - seen_targets)
    require(not missing, f"missing runtime archives for: {missing}")
    for component in ("gui", "vscode", "jetbrains"):
        require(component in versions, f"missing {component} artifact")
    return artifacts, versions


# ---------------------------------------------------------------- qualification (stable only)
INVENTORY_FILE = "tools/dev/ci_test_inventory.json"
QUALIFICATION_KIND = "refact-tests-qualification"
QUALIFICATION_NAME = "tests-qualification.json"
MAX_QUALIFICATION_BYTES = 1024 * 1024


def qualification_digest(ref: str, repository_id: int, run_id: int, run_attempt: int) -> str:
    """The artifact reference is a digest anchor, not independent network provenance."""
    match = re.fullmatch(r"github-artifact:([1-9][0-9]*)/([1-9][0-9]*)/([1-9][0-9]*)/([1-9][0-9]*)/([0-9a-f]{64})", ref or "")
    require(match is not None, "invalid qualification artifact reference")
    require(tuple(map(int, match.groups()[:3])) == (repository_id, run_id, run_attempt),
            "qualification artifact reference identity mismatch")
    return match[5]


def _load_inventory(path: Path) -> dict:
    data = json.loads(Path(path).read_text())
    require(isinstance(data, dict) and isinstance(data.get("required_jobs"), list)
            and isinstance(data.get("entries"), list), "invalid test inventory")
    for e in data["entries"]:
        require(isinstance(e, dict) and isinstance(e.get("id"), str) and isinstance(e.get("required"), bool),
                "invalid test inventory entry")
    ids = list(data["required_jobs"]) + [e["id"] for e in data["entries"]]
    require(all(isinstance(i, str) and i for i in ids) and len(ids) == len(set(ids)), "duplicate test inventory ids")
    return data


def _required_ids(inv: dict) -> set:
    return set(inv["required_jobs"]) | {e["id"] for e in inv["entries"] if e["required"]}


def verify_qualification(report_path: Path, source_sha: str, build_inputs: dict, *, root: Path,
                         repository_id: int, run_id: int, run_attempt: int, report_sha256: str,
                         trusted_inventory: Path | None = None) -> None:
    require(_digest(report_sha256, 64), "qualification sha256 required")
    try:
        require(Path(report_path).is_file() and not Path(report_path).is_symlink(), "qualification must be a regular file")
        require(0 < Path(report_path).stat().st_size <= MAX_QUALIFICATION_BYTES, "qualification byte limit exceeded")
        raw = Path(report_path).read_bytes()
        report = json.loads(raw, object_pairs_hook=_unique_object, parse_constant=_reject_constant)
    except (OSError, ValueError) as exc:
        raise ManifestError(f"unreadable qualification report: {exc}") from exc
    require(sha256_bytes(raw) == report_sha256, "qualification file digest mismatch")
    require(isinstance(report, dict) and report.get("schema_version") == 1, "qualification schema mismatch")
    require(report.get("kind") == QUALIFICATION_KIND, "qualification kind mismatch")
    for key, value in (("repository_id", repository_id), ("run_id", run_id), ("run_attempt", run_attempt)):
        require(_uint(value) and value > 0 and report.get(key) == str(value), f"qualification {key} mismatch")
    require(report.get("source_sha") == source_sha, "qualification source SHA mismatch")
    require(report.get("build_inputs") == build_inputs, "qualification build inputs mismatch")
    require(report.get("build_inputs_digest") == build_inputs_digest(build_inputs), "qualification digest mismatch")
    inventory_path = Path(root) / INVENTORY_FILE
    require(inventory_path.is_file(), f"missing {INVENTORY_FILE}")
    require(report.get("inventory_sha256") == sha256_file(inventory_path), "qualification inventory mismatch")
    inv = _load_inventory(inventory_path)
    if trusted_inventory is not None:
        trusted = _load_inventory(trusted_inventory)
        require(_required_ids(trusted) <= _required_ids(inv), "source inventory drops trusted required coverage")
    require(report.get("tests") == "passed" and report.get("status") == "PASS", "qualification did not pass")
    require(report.get("coverage_complete") is True, "qualification coverage incomplete")
    results = report.get("results")
    require(isinstance(results, list) and all(isinstance(r, dict) for r in results), "qualification has no results")
    by_id = {r.get("id"): r for r in results}
    expected = set(inv["required_jobs"]) | {e["id"] for e in inv["entries"]}
    require(len(by_id) == len(results) and set(by_id) == expected, "qualification results do not match inventory")
    for job in inv["required_jobs"]:
        require(by_id[job].get("status") == "PASS", f"required job {job} did not pass")
    for e in inv["entries"]:
        status = by_id[e["id"]].get("status")
        if e["required"]:
            require(status == "PASS", f"required entry {e['id']} did not pass")
        else:
            require(status == "PASS" or (status == "EXCLUDED" and bool(e.get("unavailable"))),
                    f"optional entry {e['id']} has invalid status")


MANIFEST_NAME = "runtime-manifest.json"


def verify_bundle(args) -> list:
    """Publication gate: the downloaded bundle must be exactly what this run's manifest describes."""
    directory = Path(args.dir)
    raw = Path(args.manifest).read_bytes()
    require(len(raw) <= MAX_MANIFEST_BYTES, "JSON byte limit exceeded")
    m = canonical_manifest(json.loads(raw))
    require(raw == compact(m), "manifest is not in canonical form")
    require(m["source_sha"] == args.source_sha, "manifest source SHA mismatch")
    require(m["build"]["channel"] == args.channel and m["build"]["version"] == args.version, "manifest build mismatch")
    require(m["workflow"]["path"] == args.workflow_path, "manifest workflow mismatch")
    require(m["run"] == {"id": args.run_id, "attempt": args.run_attempt}, "manifest run mismatch")
    require(args.channel != "stable" or m["tests"] == "passed", "stable bundle is not qualified")
    require(args.channel == "stable" or m["tests"] == "not_run", "untested bundle must declare tests not_run")
    qualification = None
    if args.channel == "stable":
        require(bool(args.root) and bool(args.trusted_inventory), "stable bundle requires source root and trusted inventory")
        root = Path(args.root)
        require(git_head(root) == m["source_sha"], "checkout HEAD does not match source SHA")
        bi = m["build_inputs"]
        require(compute_build_inputs(root, bi["rust_toolchain"], bi["node_version"], bi["profile"]) == bi,
                "source build inputs differ from manifest")
        digest = qualification_digest(m.get("qualification_ref"), m["repository"]["id"], args.run_id, args.run_attempt)
        # This receipt comes directly from Tests, not the source-controlled bundle producer.
        require(_digest(args.qualification_sha256, 64), "stable bundle requires Tests qualification sha256")
        require(re.fullmatch(r"[1-9][0-9]*", args.qualification_artifact_id or "") is not None,
                "stable bundle requires Tests qualification artifact id")
        require(args.repository_id is not None and args.repository_id > 0, "stable bundle requires repository id")
        expected_ref = (f"github-artifact:{args.repository_id}/{args.run_id}/{args.run_attempt}/"
                        f"{args.qualification_artifact_id}/{args.qualification_sha256}")
        require(m["repository"]["id"] == args.repository_id and m["qualification_ref"] == expected_ref
                and digest == args.qualification_sha256, "qualification reference differs from Tests receipt")
        qualification = directory / QUALIFICATION_NAME
        verify_qualification(qualification, m["source_sha"], bi, root=root,
                             repository_id=m["repository"]["id"], run_id=args.run_id, run_attempt=args.run_attempt,
                             report_sha256=args.qualification_sha256, trusted_inventory=Path(args.trusted_inventory))
    wanted = set(args.components.split(","))
    require(wanted and wanted <= {"runtime", "gui", "vscode", "jetbrains"}, "invalid components")
    expected = {MANIFEST_NAME}
    if qualification is not None:
        expected.add(QUALIFICATION_NAME)
    selected = []
    for a in m["artifacts"]:
        path = directory / a["filename"]
        require(path.is_file() and not path.is_symlink(), f"missing artifact {a['filename']}")
        require(path.stat().st_size == a["size"] and sha256_file(path) == a["sha256"], f"artifact digest mismatch: {a['filename']}")
        _check_sidecar(path, a["sha256"])
        expected |= {a["filename"], a["filename"] + ".sha256"}
        if a["component"] in wanted:
            selected += [str(path), str(path) + ".sha256"]
    actual = {p.name for p in directory.iterdir()}
    require(actual == expected, f"bundle contents differ from manifest: {sorted(actual ^ expected)}")
    present = {(a["component"], a["target"]) for a in m["artifacts"]}
    required = {("runtime", t) for t in DEFAULT_RUNTIME_TARGETS} | {(c, "universal") for c in ("gui", "vscode", "jetbrains")}
    require(present == required, f"bundle is not the complete release set: {sorted(present ^ required)}")
    require(selected and len(selected) == 2 * sum(1 for a in m["artifacts"] if a["component"] in wanted),
            "no publishable assets selected")
    if qualification is not None:
        selected.append(str(qualification))
    return selected


JETBRAINS_UPLOAD_URL = "https://plugins.jetbrains.com/api/updates/upload"


def classify_jetbrains_upload(status: int, body: bytes) -> None:
    """Fail closed: only a 2xx response is success; duplicates and unknown responses are errors."""
    require(200 <= status < 300, f"JetBrains Marketplace upload failed with HTTP {status}: {body[:2000]!r}")


# ---------------------------------------------------------------- producer
def load_compatibility(root: Path) -> dict:
    path = Path(root) / COMPATIBILITY_FILE
    require(path.is_file(), f"missing {COMPATIBILITY_FILE}")
    data = json.loads(path.read_text())
    require(isinstance(data, dict) and data.get("schema_version") == 1, "compatibility schema mismatch")
    comp = {k: v for k, v in data.items() if k in ("daemon_api", "worker_api", "host_bridge")}
    require(set(data) - {"schema_version", "description"} == set(comp) and len(comp) == 3,
            "compatibility file must declare exactly daemon_api, worker_api, host_bridge")
    return comp


def git_head(root: Path) -> str:
    try:
        return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    except (OSError, subprocess.CalledProcessError) as exc:
        raise ManifestError(f"cannot read git HEAD: {exc}") from exc


def produce(args) -> dict:
    root = Path(args.root).resolve()
    require(_digest(args.source_sha, 40), "source SHA must be a full lowercase 40-hex SHA")
    require(git_head(root) == args.source_sha, "checkout HEAD does not match source SHA")
    require(args.channel in CHANNELS, "invalid channel")
    require(semver(args.version), "invalid build version")
    bi = compute_build_inputs(root, args.rust_toolchain, args.node_version, args.profile)
    if args.channel == "stable":
        require(bool(args.qualification) and bool(args.qualification_ref),
                "stable channel requires --qualification and --qualification-ref")
        verify_qualification(Path(args.qualification), args.source_sha, bi, root=root,
                             repository_id=args.repository_id, run_id=args.run_id, run_attempt=args.run_attempt,
                             report_sha256=args.qualification_sha256,
                             trusted_inventory=Path(args.trusted_inventory) if args.trusted_inventory else None)
        require(qualification_digest(args.qualification_ref, args.repository_id, args.run_id, args.run_attempt)
                == args.qualification_sha256, "qualification reference digest mismatch")
        tests, qref = "passed", args.qualification_ref
    else:
        require(not args.qualification and not args.qualification_ref and not args.qualification_sha256,
                "untested channels must not carry qualification evidence")
        tests, qref = "not_run", None
    targets = tuple(args.runtime_targets.split(",")) if args.runtime_targets else DEFAULT_RUNTIME_TARGETS
    require(all(t in NATIVE_TARGETS for t in targets) and len(set(targets)) == len(targets), "invalid runtime targets")
    artifacts, versions = collect_artifacts(Path(args.artifacts_dir), args.version, targets, args.source_sha, args.channel)
    manifest = {
        "schema_version": SCHEMA_VERSION,
        "repository": {"owner": args.repository_owner, "name": args.repository_name, "id": args.repository_id},
        "source_sha": args.source_sha,
        "workflow": {"path": args.workflow_path, "id": args.workflow_id},
        "run": {"id": args.run_id, "attempt": args.run_attempt},
        "build": {"version": args.version, "channel": args.channel},
        "build_inputs": bi,
        "component_versions": {"runtime": args.version, **versions},
        "compatibility": load_compatibility(root),
        "artifacts": artifacts,
        "tests": tests,
    }
    if qref is not None:
        manifest["qualification_ref"] = qref
    return canonical_manifest(manifest)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("build-inputs")
    p.add_argument("--root", default=".")
    p.add_argument("--rust-toolchain", required=True)
    p.add_argument("--node-version", required=True)
    p.add_argument("--profile", required=True)
    p = sub.add_parser("produce")
    p.add_argument("--root", default=".")
    for name in ("source-sha", "version", "channel", "rust-toolchain", "node-version", "profile",
                 "repository-owner", "repository-name", "workflow-path", "artifacts-dir", "output"):
        p.add_argument(f"--{name}", required=True)
    for name in ("repository-id", "workflow-id", "run-id", "run-attempt"):
        p.add_argument(f"--{name}", required=True, type=int)
    p.add_argument("--runtime-targets", default="")
    p.add_argument("--qualification", default="")
    p.add_argument("--qualification-ref", default="")
    p.add_argument("--qualification-sha256", default="")
    p.add_argument("--trusted-inventory", default="")
    p = sub.add_parser("validate")
    p.add_argument("manifest")
    p = sub.add_parser("verify-qualification")
    p.add_argument("--root", default=".")
    for name in ("source-sha", "rust-toolchain", "node-version", "profile", "qualification", "qualification-sha256",
                 "trusted-inventory"):
        p.add_argument(f"--{name}", required=True)
    for name in ("repository-id", "run-id", "run-attempt"):
        p.add_argument(f"--{name}", required=True, type=int)
    p = sub.add_parser("verify-bundle")
    for name in ("manifest", "dir", "source-sha", "channel", "workflow-path", "version"):
        p.add_argument(f"--{name}", required=True)
    for name in ("run-id", "run-attempt"):
        p.add_argument(f"--{name}", required=True, type=int)
    p.add_argument("--components", default="runtime,gui,vscode,jetbrains")
    p.add_argument("--root", default="")
    p.add_argument("--trusted-inventory", default="")
    p.add_argument("--qualification-sha256", default="")
    p.add_argument("--qualification-artifact-id", default="")
    p.add_argument("--repository-id", type=int)
    p.add_argument("--assets-out", required=True)
    p = sub.add_parser("jetbrains-upload-result")
    p.add_argument("--status", required=True, type=int)
    p.add_argument("--body", required=True)
    args = parser.parse_args(argv)
    try:
        if args.cmd == "build-inputs":
            bi = compute_build_inputs(Path(args.root), args.rust_toolchain, args.node_version, args.profile)
            print(json.dumps({"build_inputs": bi, "build_inputs_digest": build_inputs_digest(bi)}))
        elif args.cmd == "produce":
            data = canonical_bytes(produce(args))
            Path(args.output).write_bytes(data)
            print(sha256_bytes(data))
        elif args.cmd == "verify-qualification":
            require(_digest(args.source_sha, 40), "source SHA must be a full lowercase 40-hex SHA")
            require(git_head(Path(args.root)) == args.source_sha, "checkout HEAD does not match source SHA")
            bi = compute_build_inputs(Path(args.root), args.rust_toolchain, args.node_version, args.profile)
            verify_qualification(Path(args.qualification), args.source_sha, bi, root=Path(args.root),
                                 repository_id=args.repository_id, run_id=args.run_id, run_attempt=args.run_attempt,
                                 report_sha256=args.qualification_sha256,
                                 trusted_inventory=Path(args.trusted_inventory))
            print(build_inputs_digest(bi))
        elif args.cmd == "verify-bundle":
            selected = verify_bundle(args)
            Path(args.assets_out).write_text("".join(f"{line}\n" for line in selected))
        elif args.cmd == "jetbrains-upload-result":
            classify_jetbrains_upload(args.status, Path(args.body).read_bytes())
        else:
            raw = Path(args.manifest).read_bytes()
            require(len(raw) <= MAX_MANIFEST_BYTES, "JSON byte limit exceeded")
            print(manifest_digest(json.loads(raw)))
    except (ManifestError, ValueError, KeyError, OSError, tarfile.TarError, zipfile.BadZipFile, ET.ParseError) as exc:
        print(f"runtime_manifest: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())

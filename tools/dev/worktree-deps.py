#!/usr/bin/env python3
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import stat
import sys
from typing import TypedDict


SCHEMA = 1
RECEIPT = ".refact-deps-receipt.json"
EXCLUDED = {
    ".cache", ".vite", ".vite-temp", ".vitest", ".turbo", ".parcel-cache",
    ".eslintcache", ".rpt2_cache", ".tscache", ".swc", ".rollup.cache",
    ".webpack", ".next", ".nuxt", ".svelte-kit", "coverage", ".nyc_output",
}


class Environment(TypedDict):
    node: str
    node_abi: str
    platform: str
    arch: str
    libc: str
    npm: str
    install_policy: str


class SourceReceipt(TypedDict):
    schema: int
    kind: str
    producer: str
    install_id: str
    source: str
    inputs: dict
    key: str
    tree_sha256: str


class CheckoutReceipt(TypedDict):
    schema: int
    kind: str
    owner_id: str
    owner: dict


class Refusal(ValueError):
    pass


def digest(value):
    return hashlib.sha256(value).hexdigest()


def encoded(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def canonical(value, directory=True):
    path = Path(value)
    if not path.is_absolute() or path != path.resolve(strict=True):
        raise Refusal(f"Use an absolute canonical path without symlinks: {path}")
    info = path.lstat()
    if directory and not stat.S_ISDIR(info.st_mode):
        raise Refusal(f"Not a real directory: {path}")
    if info.st_uid != os.getuid() or info.st_mode & 0o022:
        raise Refusal(f"Must be caller-owned and not group/world writable: {path}")
    return path


def directory_fd(path):
    if os.name != "posix" or not hasattr(os, "O_NOFOLLOW") or not hasattr(os, "O_DIRECTORY"):
        raise Refusal("Unsupported platform: POSIX no-follow directory descriptors are required")
    path = Path(path)
    if not path.is_absolute() or ".." in path.parts:
        raise Refusal(f"Absolute non-traversing path required: {path}")
    fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in path.parts[1:]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
        return fd
    except Exception:
        os.close(fd)
        raise


def regular_bytes(path):
    parent = directory_fd(path.parent)
    try:
        fd = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
    finally:
        os.close(parent)
    with os.fdopen(fd, "rb") as stream:
        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
            raise Refusal(f"Not a regular file: {path}")
        return stream.read()


def json_file(path):
    return json.loads(regular_bytes(path))


def inputs(package: Path, environment: Environment):
    required = {"node", "node_abi", "platform", "arch", "libc", "npm", "install_policy"}
    if not isinstance(environment, dict) or set(environment) != required or any(not isinstance(value, str) or not value.strip() for value in environment.values()):
        raise Refusal(f"Environment must contain nonempty strings: {sorted(required)}")
    host = {"linux": "linux", "darwin": "darwin", "win32": "win32"}.get(sys.platform)
    machine = {"x86_64": "x64", "AMD64": "x64", "aarch64": "arm64", "arm64": "arm64"}.get(platform.machine())
    if environment["platform"] != host or environment["arch"] != machine:
        raise Refusal("Environment platform/arch do not match this host; no cross-platform reuse")
    files = {}
    for name in ("package.json", "package-lock.json", ".npmrc"):
        path = package / name
        if name == ".npmrc" and not os.path.lexists(path):
            files[name] = None
        else:
            files[name] = digest(regular_bytes(path))
    manifest = json_file(package / "package.json")
    lock = json_file(package / "package-lock.json")
    if not isinstance(manifest, dict) or not isinstance(lock, dict) or not isinstance(lock.get("packages"), dict):
        raise Refusal("Expected package/lockfile JSON objects with a packages map")
    root = lock["packages"].get("")
    if lock.get("lockfileVersion") not in (2, 3) or not isinstance(root, dict):
        raise Refusal("An npm lockfile v2/v3 with a root package is required")
    for field in ("name", "version", "dependencies", "devDependencies", "optionalDependencies"):
        if manifest.get(field) != root.get(field):
            raise Refusal(f"package.json and package-lock.json disagree on {field}")
    patches = package / "patches"
    if os.path.lexists(patches):
        canonical(patches)
        for parent, dirs, names in os.walk(patches, followlinks=False):
            for name in dirs:
                canonical(Path(parent) / name)
            for name in sorted(names):
                path = Path(parent) / name
                files[path.relative_to(package).as_posix()] = digest(regular_bytes(path))
    record = {"schema": SCHEMA, "files": files, "environment": environment, "copy_policy": "private-regular-files-and-internal-file-links-v1"}
    return record, digest(encoded(record))


def excluded(relative):
    return any(part in EXCLUDED or part.startswith(".vite") or part.endswith(".tsbuildinfo") for part in relative.parts)


def inventory(root):
    root = canonical(root)
    entries = []

    def walk(directory, relative_parent):
        for name in sorted(os.listdir(directory)):
            relative = relative_parent / name
            path = root / relative
            if relative == Path(RECEIPT) or excluded(relative):
                continue
            if relative == Path("node_modules"):
                continue
            info = os.stat(name, dir_fd=directory, follow_symlinks=False)
            if info.st_uid != os.getuid() or (not stat.S_ISLNK(info.st_mode) and info.st_mode & 0o022):
                raise Refusal(f"Dependency entry is not privately owned: {path}")
            if stat.S_ISDIR(info.st_mode):
                entries.append({"path": relative.as_posix(), "kind": "directory"})
                child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory)
                try:
                    walk(child, relative)
                finally:
                    os.close(child)
            elif stat.S_ISREG(info.st_mode):
                entries.append({"path": relative.as_posix(), "kind": "file", "sha256": digest(regular_bytes(path)), "executable": bool(info.st_mode & 0o111)})
            elif stat.S_ISLNK(info.st_mode):
                link = os.readlink(name, dir_fd=directory)
                if Path(link).is_absolute():
                    raise Refusal(f"Absolute dependency link refused: {path}")
                target = Path(os.path.normpath(path.parent / link))
                if not target.is_relative_to(root):
                    raise Refusal(f"Escaping dependency link refused: {path}")
                target_relative = target.relative_to(root)
                if not target_relative.parts or excluded(target_relative) or target_relative.parts[0] == "node_modules" or target_relative == Path(RECEIPT):
                    raise Refusal(f"Dependency link points to excluded data: {path}")
                entries.append({"path": relative.as_posix(), "kind": "symlink", "target": link, "resolved": target_relative.as_posix()})
            else:
                raise Refusal(f"Special dependency entry refused: {path}")

    root_fd = directory_fd(root)
    try:
        walk(root_fd, Path())
    finally:
        os.close(root_fd)
    by_path = {entry["path"]: entry for entry in entries}
    for entry in entries:
        if entry["kind"] != "symlink":
            continue
        components = list(Path(entry["path"]).parent.parts)
        parts = Path(entry["target"]).parts
        for index, part in enumerate(parts):
            if part == "..":
                if not components:
                    raise Refusal(f"Escaping dependency link refused: {entry['path']}")
                components.pop()
            else:
                components.append(part)
            if index < len(parts) - 1 and components and by_path.get("/".join(components), {}).get("kind") != "directory":
                raise Refusal(f"Dependency link traverses a non-directory: {entry['path']}")
        if by_path.get(entry["resolved"], {}).get("kind") != "file":
            raise Refusal(f"Link does not terminate in an included regular file: {entry['path']}")
    return entries


def destination(checkout, package):
    checkout = canonical(checkout)
    package = canonical(package)
    if not package.is_relative_to(checkout):
        raise Refusal("Destination package escapes the explicitly owned checkout")
    if not (checkout / ".git").is_file() or (checkout / ".git").is_symlink():
        raise Refusal("Destination must be an isolated linked Git checkout, not the main directory")
    marker = regular_bytes(checkout / ".git").decode().strip()
    if not marker.startswith("gitdir: "):
        raise Refusal("Invalid linked checkout identity")
    gitdir = (checkout / marker[8:]).resolve(strict=True)
    if regular_bytes(gitdir / "gitdir").decode().strip() != str(checkout / ".git"):
        raise Refusal("Linked Git checkout back-reference mismatch")
    output = package / "node_modules"
    if output.is_symlink():
        raise Refusal("Destination node_modules is a symlink; preserved, never copy through it")
    return output, {"checkout": str(checkout), "package": str(package), "gitdir": str(gitdir), "device": checkout.stat().st_dev, "inode": checkout.stat().st_ino}


def pinned_receipt(path, sha256):
    path = canonical(path, directory=False)
    data = regular_bytes(path)
    if digest(data) != sha256:
        raise Refusal("Trusted receipt SHA256 mismatch")
    receipt = json.loads(data)
    if not isinstance(receipt, dict):
        raise Refusal("Trusted receipt must be a JSON object")
    return receipt


def prepare(checkout, package, source, receipt_path, receipt_sha256, environment,
            ownership_path, ownership_sha256, apply=False):
    output, owner = destination(checkout, package)
    ownership = pinned_receipt(ownership_path, ownership_sha256)
    if (set(ownership) != set(CheckoutReceipt.__annotations__)
            or ownership.get("schema") != SCHEMA or ownership.get("kind") != "refact-owned-checkout"
            or ownership.get("owner") != owner or not isinstance(ownership.get("owner_id"), str)
            or not ownership["owner_id"]):
        raise Refusal("Trusted checkout receipt does not authorize this checkout identity/package")
    source = canonical(source)
    source_modules = canonical(source / "node_modules")
    if output.is_relative_to(source_modules) or source_modules.is_relative_to(output):
        raise Refusal("Source and destination overlap")
    receipt = pinned_receipt(receipt_path, receipt_sha256)
    source_inputs, source_key = inputs(source, environment)
    target_inputs, target_key = inputs(Path(package), environment)
    if source_key != target_key:
        raise Refusal("Dependency inputs differ (manifest/lockfile/patches/environment); prepare a matching trusted source offline")
    if (set(receipt) != set(SourceReceipt.__annotations__)
            or receipt.get("schema") != SCHEMA or receipt.get("kind") != "refact-managed-dependency-install"
            or not isinstance(receipt.get("producer"), str) or not receipt["producer"]
            or not isinstance(receipt.get("install_id"), str) or not receipt["install_id"]
            or receipt.get("source") != str(source) or receipt.get("inputs") != source_inputs
            or receipt.get("key") != source_key):
        raise Refusal("Source install receipt does not bind these current inputs; do not borrow stale installed dependencies")
    entries = inventory(source_modules)
    tree_digest = digest(encoded(entries))
    if not entries or receipt.get("tree_sha256") != tree_digest:
        raise Refusal("Source install receipt dependency-tree mismatch or empty dependencies")
    expected = {"schema": SCHEMA, "owner": owner, "owner_id": ownership["owner_id"], "ownership_receipt_sha256": ownership_sha256, "source": str(source), "source_receipt": str(receipt_path), "source_receipt_sha256": receipt_sha256, "key": target_key, "inputs": target_inputs, "tree_sha256": tree_digest}
    if os.path.lexists(output):
        canonical(output)
        try:
            actual = json_file(output / RECEIPT)
        except (OSError, ValueError) as error:
            raise Refusal("Existing node_modules has no readable ownership receipt; preserved. Choose a fresh checkout or have its owner move it explicitly") from error
        if actual != expected or digest(encoded(inventory(output))) != tree_digest:
            raise Refusal("Existing node_modules lacks matching ownership/content receipt; preserved. Choose a fresh checkout or have its owner move it explicitly")
        return {"status": "already-prepared", **expected}
    if not apply:
        return {"status": "dry-run", **expected}
    parent_fd = directory_fd(output.parent)
    try:
        os.mkdir(output.name, mode=0o700, dir_fd=parent_fd)
        output_fd = os.open(output.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent_fd)
    finally:
        os.close(parent_fd)
    try:
        for entry in entries:
            source_path = source_modules / entry["path"]
            target_path = output / entry["path"]
            parent = directory_fd(target_path.parent)
            try:
                if entry["kind"] == "directory":
                    os.mkdir(target_path.name, mode=0o700, dir_fd=parent)
                elif entry["kind"] == "file":
                    data = regular_bytes(source_path)
                    if digest(data) != entry["sha256"]:
                        raise Refusal("Source changed during copy; incomplete owned destination retained")
                    fd = os.open(target_path.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o700 if entry["executable"] else 0o600, dir_fd=parent)
                    with os.fdopen(fd, "wb") as stream:
                        stream.write(data)
            finally:
                os.close(parent)
        for entry in entries:
            if entry["kind"] == "symlink":
                target_path = output / entry["path"]
                parent = directory_fd(target_path.parent)
                try:
                    os.symlink(entry["target"], target_path.name, dir_fd=parent)
                finally:
                    os.close(parent)
        if digest(encoded(inventory(output))) != tree_digest or digest(encoded(inventory(source_modules))) != tree_digest:
            raise Refusal("Dependency content changed during preparation; incomplete owned destination retained")
        if inputs(source, environment) != (source_inputs, source_key) or inputs(Path(package), environment) != (target_inputs, target_key):
            raise Refusal("Dependency inputs changed during preparation; incomplete owned destination retained")
        if destination(checkout, package) != (output, owner) or output.stat().st_ino != os.fstat(output_fd).st_ino:
            raise Refusal("Checkout/output identity changed during preparation")
        fd = os.open(RECEIPT, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=output_fd)
        with os.fdopen(fd, "wb") as stream:
            stream.write(encoded(expected))
            stream.flush()
            os.fsync(stream.fileno())
    except Exception:
        print(f"Incomplete destination retained without cleanup: {output}; inspect before any explicit owner cleanup", file=sys.stderr)
        raise
    finally:
        os.close(output_fd)
    return {"status": "prepared", **expected}


def main():
    parser = argparse.ArgumentParser(description="Offline, receipt-bound dependency copy. No installs, scripts, subprocesses, shared writable dependencies or automatic cleanup.")
    parser.add_argument("--checkout", required=True, type=Path, help="Explicitly authorized canonical isolated checkout; not a path grant")
    parser.add_argument("--package", required=True, type=Path)
    parser.add_argument("--source", required=True, type=Path, help="Explicit trusted canonical package directory; read-only")
    parser.add_argument("--source-receipt", required=True, type=Path)
    parser.add_argument("--source-receipt-sha256", required=True, help="Digest obtained independently from the trusted install producer")
    parser.add_argument("--ownership-receipt", required=True, type=Path, help="Trusted checkout controller receipt; not minted by this consumer")
    parser.add_argument("--ownership-receipt-sha256", required=True)
    parser.add_argument("--environment", required=True, type=Path, help="Trusted target runtime/install-policy JSON; no automatic Node/npm execution")
    parser.add_argument("--apply", action="store_true", help="Create an absent checkout-owned node_modules; default is dry-run")
    args = parser.parse_args()
    result = prepare(args.checkout, args.package, args.source, args.source_receipt, args.source_receipt_sha256, json_file(args.environment), args.ownership_receipt, args.ownership_receipt_sha256, args.apply)
    print(json.dumps(result, sort_keys=True, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError) as error:
        sys.exit(f"worktree-deps: {error}. Offline refusal: nothing is installed or automatically removed.")

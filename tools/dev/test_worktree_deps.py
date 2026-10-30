import importlib.util
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile
import unittest


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("worktree_deps", HERE / "worktree-deps.py")
deps = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(deps)


class DependencyPreparation(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.checkout = self.root / "checkout"
        self.package = self.checkout / "gui"
        self.source = self.root / "managed-install"
        self.package.mkdir(parents=True)
        self.source.mkdir()
        gitdir = self.root / "gitdir"
        gitdir.mkdir()
        (gitdir / "gitdir").write_text(str(self.checkout / ".git") + "\n")
        (self.checkout / ".git").write_text(f"gitdir: {gitdir}\n")
        manifest = {"name": "fixture", "version": "1.0.0", "dependencies": {"fixture-bin": "1.0.0"}}
        lock = {"lockfileVersion": 3, "packages": {"": manifest, "node_modules/fixture-bin": {"version": "1.0.0"}}}
        for package in (self.package, self.source):
            (package / "package.json").write_text(json.dumps(manifest))
            (package / "package-lock.json").write_text(json.dumps(lock))
        self.modules = self.source / "node_modules"
        (self.modules / "fixture-bin").mkdir(parents=True)
        (self.modules / "fixture-bin" / "cli.js").write_text("fixture data only\n")
        (self.modules / "fixture-bin" / "cli.js").chmod(0o755)
        (self.modules / ".bin").mkdir()
        (self.modules / ".bin" / "fixture").symlink_to("../fixture-bin/cli.js")
        (self.modules / "node_modules").symlink_to(".")
        (self.modules / ".vite").mkdir()
        (self.modules / ".vite" / "cached.js").write_text("mutable cache")
        (self.modules / "fixture.tsbuildinfo").write_text("mutable cache")
        self.environment = {"node": "v22.fixture", "node_abi": "fixture-abi", "platform": sys.platform, "arch": {"x86_64": "x64", "AMD64": "x64", "aarch64": "arm64", "arm64": "arm64"}[platform.machine()], "libc": "fixture-libc", "npm": "fixture-npm", "install_policy": "fixture-no-scripts"}
        self.receipt = self.root / "source-receipt.json"
        record, key = deps.inputs(self.source, self.environment)
        receipt = {"schema": 1, "kind": "refact-managed-dependency-install", "producer": "temporary-fixture", "install_id": "fixture-only", "source": str(self.source), "inputs": record, "key": key, "tree_sha256": deps.digest(deps.encoded(deps.inventory(self.modules)))}
        self.receipt.write_bytes(deps.encoded(receipt))
        self.receipt_hash = deps.digest(self.receipt.read_bytes())
        self.ownership = self.root / "ownership.json"
        _, owner = deps.destination(self.checkout, self.package)
        self.ownership.write_bytes(deps.encoded({"schema": 1, "kind": "refact-owned-checkout", "owner_id": "temporary-fixture", "owner": owner}))
        self.ownership_hash = deps.digest(self.ownership.read_bytes())

    def prepare(self, **changes):
        arguments = {"checkout": self.checkout, "package": self.package, "source": self.source, "receipt_path": self.receipt, "receipt_sha256": self.receipt_hash, "environment": self.environment, "ownership_path": self.ownership, "ownership_sha256": self.ownership_hash, "apply": True}
        arguments.update(changes)
        return deps.prepare(**arguments)

    def test_private_copy_excludes_selflink_and_caches_preserves_bin_and_is_idempotent(self):
        self.assertEqual(self.prepare(apply=False)["status"], "dry-run")
        output = self.package / "node_modules"
        self.assertFalse(output.exists())
        self.assertEqual(self.prepare()["status"], "prepared")
        self.assertEqual(self.prepare()["status"], "already-prepared")
        self.assertFalse(os.path.lexists(output / "node_modules"))
        self.assertFalse((output / ".vite").exists())
        self.assertFalse((output / "fixture.tsbuildinfo").exists())
        self.assertEqual(os.readlink(output / ".bin" / "fixture"), "../fixture-bin/cli.js")
        self.assertEqual((output / ".bin" / "fixture").read_bytes(), (self.modules / "fixture-bin" / "cli.js").read_bytes())
        (output / "fixture-bin" / "cli.js").write_text("local mutation")
        self.assertEqual((self.modules / "fixture-bin" / "cli.js").read_text(), "fixture data only\n")
        with self.assertRaisesRegex(deps.Refusal, "preserved"):
            self.prepare()

    def test_lockfile_and_trusted_receipt_mismatches_refuse_before_output(self):
        with self.assertRaisesRegex(deps.Refusal, "SHA256 mismatch"):
            self.prepare(receipt_sha256="0" * 64)
        with self.assertRaisesRegex(deps.Refusal, "SHA256 mismatch"):
            self.prepare(ownership_sha256="0" * 64)
        lock = self.package / "package-lock.json"
        original = lock.read_text()
        lock.write_text(original + "\n")
        with self.assertRaisesRegex(deps.Refusal, "inputs differ"):
            self.prepare()
        lock.write_text(original)
        patches = self.package / "patches"
        patches.mkdir()
        (patches / "new.patch").write_text("different patched install")
        with self.assertRaisesRegex(deps.Refusal, "inputs differ"):
            self.prepare()
        (patches / "new.patch").unlink()
        with self.assertRaisesRegex(deps.Refusal, "does not bind"):
            self.prepare(environment={**self.environment, "node_abi": "different-abi"})
        (self.modules / "fixture-bin" / "cli.js").write_text("stale installed content")
        with self.assertRaisesRegex(deps.Refusal, "dependency-tree mismatch"):
            self.prepare()
        self.assertFalse((self.package / "node_modules").exists())

    def test_escaping_destination_source_links_and_existing_user_outputs_are_preserved(self):
        output = self.package / "node_modules"
        output.symlink_to(self.modules)
        with self.assertRaisesRegex(deps.Refusal, "symlink"):
            self.prepare()
        self.assertTrue(output.is_symlink())
        output.unlink()
        outside = self.root / "outside"
        outside.mkdir()
        with self.assertRaisesRegex(deps.Refusal, "escapes"):
            self.prepare(package=outside)
        (self.modules / "escape").symlink_to("../../outside")
        with self.assertRaisesRegex(deps.Refusal, "Escaping"):
            self.prepare()
        (self.modules / "escape").unlink()
        output.mkdir()
        sentinel = output / "user-data"
        sentinel.write_text("preserve me")
        with self.assertRaises((deps.Refusal, OSError)):
            self.prepare()
        self.assertEqual(sentinel.read_text(), "preserve me")


class CargoCacheSetup(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.cwd = self.root / "checkout"
        self.home = self.root / "cargo-home"
        self.cwd.mkdir()
        self.home.mkdir()
        self.config = self.home / "config.toml"
        self.env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": str(self.root), "SETUP_CACHE_PYTHON": sys.executable}
        try:
            import tomllib
        except ImportError:
            try:
                import tomli as tomllib
            except ImportError:
                self.fail("Central fixture execution requires an already-installed Python 3.11+ or tomli; no parser is installed automatically")
        self.toml = tomllib

    def run_setup(self, *arguments):
        return subprocess.run(["bash", str(HERE / "setup-cache.sh"), "--cwd", str(self.cwd), "--cargo-home", str(self.home), *arguments], env=self.env, capture_output=True, text=True, check=False)

    def test_commented_wrapper_table_preservation_and_idempotent_atomic_apply(self):
        original = '# rustc-wrapper = "old"\n[build]\njobs = 3\n[target.fixture]\nrustflags = ["-C", "debuginfo=0"]\n'
        self.config.write_text(original)
        status = self.run_setup("--status")
        self.assertEqual(status.returncode, 0, status.stderr)
        self.assertIn("unset (not enabled)", status.stdout)
        self.assertEqual(self.config.read_text(), original)
        dry = self.run_setup("--dry-run")
        self.assertEqual(dry.returncode, 0, dry.stderr)
        self.assertEqual(self.config.read_text(), original)
        applied = self.run_setup("--apply")
        self.assertEqual(applied.returncode, 0, applied.stderr)
        expected = self.toml.loads(original)
        expected["build"].update({"rustc-wrapper": "sccache", "incremental": False})
        updated = self.config.read_text()
        self.assertEqual(self.toml.loads(updated), expected)
        backups = list(self.home.glob("config.toml.bak.*"))
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_text(), original)
        again = self.run_setup("--apply")
        self.assertEqual(again.returncode, 0, again.stderr)
        self.assertEqual(self.config.read_text(), updated)
        self.assertEqual(list(self.home.glob("config.toml.bak.*")), backups)
        self.assertFalse((self.root / ".bashrc").exists())

    def test_custom_and_higher_precedence_wrappers_are_not_replaced(self):
        original = '[build]\nrustc-wrapper = "custom-wrapper"\njobs = 2\n'
        self.config.write_text(original)
        refused = self.run_setup("--apply")
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("Custom rustc-wrapper preserved", refused.stderr)
        self.assertEqual(self.config.read_text(), original)
        self.config.write_text("[build]\njobs = 2\n")
        (self.cwd / ".cargo").mkdir()
        local = self.cwd / ".cargo" / "config.toml"
        local.write_text('[build]\nrustc-wrapper = "local-custom"\n')
        refused = self.run_setup("--apply")
        self.assertNotEqual(refused.returncode, 0)
        self.assertEqual(self.config.read_text(), "[build]\njobs = 2\n")
        self.assertEqual(local.read_text(), '[build]\nrustc-wrapper = "local-custom"\n')
        self.env["RUSTC_WRAPPER"] = "sccache"
        self.assertNotEqual(self.run_setup("--apply").returncode, 0)
        local.write_text('[build]\nrustc-wrapper = "sccache"\n')
        self.config.write_text('build = { jobs = 2 }\n')
        refused = self.run_setup("--apply")
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("Cannot safely extend", refused.stderr)
        self.assertEqual(self.config.read_text(), 'build = { jobs = 2 }\n')


if __name__ == "__main__":
    unittest.main()

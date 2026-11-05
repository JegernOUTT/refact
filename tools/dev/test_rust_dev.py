import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("rust_dev", Path(__file__).with_name("rust-dev.py"))
dev = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dev)


class RustDevTests(unittest.TestCase):
    def test_default_check_and_build(self):
        command, env, warnings, errors, channel = dev.plan(dev.arguments([]), {})
        self.assertEqual(command[:5], ["cargo", "check", "-p", "refact-lsp", "--lib"])
        self.assertIn("--locked", command)
        self.assertIn("--offline", command)
        self.assertEqual(env, {"CARGO_INCREMENTAL": "1", "REFACT_SKIP_GUI_BUILD": "1", "RUSTUP_AUTO_INSTALL": "0"})
        self.assertEqual(command[command.index("--target-dir") + 1], str(dev.ENGINE / "target"))
        self.assertIsNone(channel)
        self.assertFalse(errors)
        self.assertIn("unvalidated", warnings[0])
        build = dev.plan(dev.arguments(["--mode", "build", "--toolchain", "stable", "--online"]), {})[0]
        self.assertEqual(build[:7], ["cargo", "+stable", "build", "-p", "refact-lsp", "--bin", "refact-lsp"])
        self.assertNotIn("--offline", build)

    @patch.object(dev, "available_memory", return_value=dev.MIN_MEMORY)
    def test_experimental_root_flags_and_target_isolation(self, memory):
        llvm = dev.plan(dev.arguments(["--mode", "experimental-lib"]), {})[0]
        clif = dev.plan(dev.arguments(["--mode", "experimental-lib", "--cranelift"]), {})[0]
        self.assertEqual(llvm[:6], ["cargo", "+nightly-2026-11-04", "rustc", "-p", "refact-lsp", "--lib"])
        self.assertEqual(llvm[llvm.index("--") + 1:], ["-Zunstable-options", "--jobs-frontend=4"])
        self.assertEqual(clif[-1], "-Zcodegen-backend=cranelift")
        self.assertNotEqual(llvm[llvm.index("--target-dir") + 1], clif[clif.index("--target-dir") + 1])
        self.assertNotIn("RUSTFLAGS", dev.plan(dev.arguments([]), {})[1])

    def test_argument_restrictions_and_exact_features(self):
        args = dev.arguments(["--package", "refact-core", "--", "--features", "a,b", "--no-default-features"])
        command = dev.plan(args, {})[0]
        self.assertEqual(command[-3:], ["--features", "a,b", "--no-default-features"])
        for args in (["--jobs", "0"], ["--cranelift"], ["--toolchain", dev.NIGHTLY],
                     ["--mode", "build", "--package", "other"], ["--", "--target-dir", "/tmp"],
                     ["--", "--config", "build.rustflags=[]"], ["--", "--features"]):
            with self.subTest(args=args), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                dev.arguments(args)

    @patch.object(dev, "available_memory", return_value=1024)
    @patch.object(dev.subprocess, "run")
    def test_low_memory_refusal_override_and_dry_run(self, run, memory):
        with self.assertRaisesRegex(ValueError, "RAM"), contextlib.redirect_stderr(io.StringIO()):
            dev.main(["--mode", "experimental-lib"])
        run.assert_not_called()
        self.assertFalse(dev.plan(dev.arguments(["--mode", "experimental-lib", "--allow-low-memory"]), {})[3])
        output = io.StringIO()
        with contextlib.redirect_stdout(output), patch.object(Path, "mkdir") as mkdir, patch.object(Path, "open") as open_file:
            self.assertEqual(dev.main(["--mode", "experimental-lib", "--dry-run"]), 0)
        self.assertTrue(json.loads(output.getvalue())["execution_blockers"])
        run.assert_not_called()
        mkdir.assert_not_called()
        open_file.assert_not_called()

    def test_wrappers_bootstrap_and_parent_environment_unchanged(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(dev, "ENGINE", Path(directory)), \
                patch.dict(os.environ, {"RUSTC_WRAPPER": "privacy transport", "RUSTC_WORKSPACE_WRAPPER": "ide-wrapper", "SECRET": "not-printed"}, clear=True):
            before = dict(os.environ)
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaisesRegex(ValueError, "wrapper"):
                dev.main([])
            with patch.object(dev.subprocess, "run", side_effect=[subprocess.CompletedProcess([], 0), subprocess.CompletedProcess([], 7)]) as run, contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(dev.main(["--allow-wrapper"]), 7)
            env = run.call_args.kwargs["env"]
            self.assertEqual(env["RUSTC_WRAPPER"], "privacy transport")
            self.assertEqual(env["RUSTC_WORKSPACE_WRAPPER"], "ide-wrapper")
            self.assertEqual(dict(os.environ), before)
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                dev.main(["--dry-run"])
            self.assertNotIn("not-printed", output.getvalue())
        self.assertTrue(dev.plan(dev.arguments([]), {"RUSTC_BOOTSTRAP": "1"})[3])

    @patch.object(dev.subprocess, "run", return_value=subprocess.CompletedProcess([], 1))
    def test_missing_toolchain_never_runs_cargo_or_creates_target(self, run):
        with tempfile.TemporaryDirectory() as directory, patch.object(dev, "ENGINE", Path(directory)), \
                patch.dict(os.environ, {}, clear=True), contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaisesRegex(ValueError, "Install explicitly"):
                dev.main(["--toolchain", "stable"])
            self.assertFalse((Path(directory) / "target").exists())
        self.assertEqual(run.call_count, 1)
        self.assertEqual(run.call_args.args[0], ["rustup", "which", "--toolchain", "stable", "cargo"])

    def test_checkout_lock_excludes_second_helper(self):
        import fcntl
        with tempfile.TemporaryDirectory() as directory, patch.object(dev, "ENGINE", Path(directory)), \
                patch.dict(os.environ, {}, clear=True), contextlib.redirect_stderr(io.StringIO()):
            target = Path(directory) / "target"
            target.mkdir()
            with (target / ".rust-dev.lock").open("a") as lock:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                with patch.object(dev.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as run:
                    with self.assertRaisesRegex(ValueError, "another rust-dev"):
                        dev.main([])
                    self.assertEqual(run.call_count, 1)

    def test_memory_probe_honors_cgroup_headroom_and_unknown_refusal(self):
        files = self.memory_files()
        files["/sys/fs/cgroup/team/worker/memory.max"] = str(8 * 1024**3)
        files["/sys/fs/cgroup/team/worker/memory.current"] = str(1024**3)
        with self.mock_memory_files(files):
            self.assertEqual(dev.available_memory(), 7 * 1024**3)
            self.assertTrue(dev.plan(dev.arguments(["--mode", "experimental-lib"]), {})[3])

    @staticmethod
    def memory_files():
        return {"/proc/meminfo": "MemAvailable: 67108864 kB\n",
                "/proc/self/cgroup": "0::/team/worker\n",
                "/proc/self/mountinfo": "30 20 0:27 / /sys/fs/cgroup rw - cgroup2 cgroup rw\n",
                "/sys/fs/cgroup/memory.max": "max",
                "/sys/fs/cgroup/team/memory.max": "max",
                "/sys/fs/cgroup/team/worker/memory.max": "max"}

    @staticmethod
    def mock_memory_files(files):
        def read(path, *args, **kwargs):
            try:
                return files[str(path)]
            except KeyError:
                raise FileNotFoundError(str(path)) from None
        return patch.object(Path, "read_text", autospec=True, side_effect=read)

    def test_memory_probe_effective_ancestor_headroom(self):
        files = self.memory_files()
        files["/sys/fs/cgroup/team/memory.max"] = str(50 * 1024**3)
        files["/sys/fs/cgroup/team/memory.current"] = str(10 * 1024**3)
        with self.mock_memory_files(files):
            self.assertEqual(dev.available_memory(), 40 * 1024**3)

    def test_memory_probe_metadata_missing_cannot_fall_back_to_root(self):
        for missing in ("/proc/self/cgroup", "/proc/self/mountinfo",
                        "/sys/fs/cgroup/team/memory.max"):
            files = self.memory_files()
            del files[missing]
            with self.subTest(missing=missing), self.mock_memory_files(files):
                self.assertIsNone(dev.available_memory())
                self.assertTrue(dev.plan(dev.arguments(["--mode", "experimental-lib"]), {})[3])

    def test_memory_probe_unsupported_mounts_membership_and_symlinks(self):
        for path, value in (("/proc/self/cgroup", "5:memory:/team/worker\n"),
                            ("/proc/self/cgroup", "0::/../escape\n"),
                            ("/proc/self/mountinfo", "30 20 0:27 /team /sys/fs/cgroup rw - cgroup2 cgroup rw\n")):
            files = self.memory_files()
            files[path] = value
            with self.subTest(value=value), self.mock_memory_files(files):
                self.assertIsNone(dev.available_memory())
        with self.mock_memory_files(self.memory_files()), patch.object(Path, "resolve", return_value=Path("/elsewhere")):
            self.assertIsNone(dev.available_memory())

    def test_cargo_build_wrapper_environment_requires_explicit_passthrough(self):
        for key in ("CARGO_BUILD_RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER"):
            with self.subTest(key=key), tempfile.TemporaryDirectory() as directory, \
                    patch.object(dev, "ENGINE", Path(directory)), patch.dict(os.environ, {key: "transport"}, clear=True):
                with contextlib.redirect_stderr(io.StringIO()), self.assertRaisesRegex(ValueError, "wrapper"):
                    dev.main([])
                with patch.object(dev.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as run, contextlib.redirect_stderr(io.StringIO()):
                    self.assertEqual(dev.main(["--allow-wrapper"]), 0)
                self.assertEqual(run.call_args.kwargs["env"][key], "transport")
                self.assertEqual(os.environ[key], "transport")

    def test_symlink_target_outside_checkout_refused(self):
        with tempfile.TemporaryDirectory() as directory, tempfile.TemporaryDirectory() as outside:
            (Path(directory) / "target").symlink_to(outside, target_is_directory=True)
            with patch.object(dev, "ENGINE", Path(directory)), self.assertRaisesRegex(ValueError, "outside"):
                dev.plan(dev.arguments([]), {})


if __name__ == "__main__":
    unittest.main()

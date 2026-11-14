"""Real Cargo regression tests; Python 3.10+ with tomli or 3.11+, Linux, cached dependencies.

Run: python3 tools/dev/test_build_freshness.py -v
Before-fix witness: REFACT_BUILD_SCRIPT_REF=<commit> python3 tools/dev/test_build_freshness.py BuildFreshness.test_default_warm_build_and_run -v
Only build.rs is overridden by the optional Git ref; artifact support stays shipping code.
All disposable repositories and compilation outputs live under engine/target.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

try:
    import tomllib
except ImportError:
    import tomli as tomllib


ROOT = Path(__file__).resolve().parents[2]
ENGINE = ROOT / "refact-agent/engine"
SCRATCH = ENGINE / "target/build-freshness-tests"
CONTROLLER = Path("/etc/refact/worktree-cache/gui-controller.json")
NAME = "freshness_probe"


def execute(args, cwd, env=None, timeout=240):
    return subprocess.run(args, cwd=cwd, env=env, text=True, capture_output=True,
                          timeout=timeout, check=False)


def toml_value(value):
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{json.dumps(k)} = {toml_value(v)}" for k, v in value.items()) + " }"
    return json.dumps(value)


def current_build_out_dir(stdout, manifest):
    """Bind this invocation's fixture binary to its exact build-script output."""
    messages = []
    for line in stdout.splitlines():
        try:
            item = json.loads(line)
        except ValueError:
            continue
        if isinstance(item, dict):
            messages.append(item)
    artifacts = [item for item in messages
                 if item.get("reason") == "compiler-artifact"
                 and item.get("manifest_path") == str(manifest)
                 and item.get("target", {}).get("name") == NAME
                 and "bin" in item.get("target", {}).get("kind", [])]
    if len(artifacts) != 1 or not artifacts[0].get("package_id"):
        raise ValueError(f"expected one current {NAME} binary artifact: {artifacts!r}")
    scripts = [item for item in messages
               if item.get("reason") == "build-script-executed"
               and item.get("package_id") == artifacts[0]["package_id"]]
    if len(scripts) != 1 or not scripts[0].get("out_dir"):
        raise ValueError(f"expected one current {NAME} build-script output: {scripts!r}")
    return Path(scripts[0]["out_dir"])


class BuildOutputSelectionTests(unittest.TestCase):
    """Cargo-free regressions; run this class without BuildFreshness setup."""

    def setUp(self):
        self.manifest = Path("/fixture/current/engine/Cargo.toml")
        self.package = "path+file:///fixture/current/engine#freshness_probe@0.1.0"
        self.artifact = {"reason": "compiler-artifact", "manifest_path": str(self.manifest),
                         "package_id": self.package, "target": {"name": NAME, "kind": ["bin"]}}

    def select(self, messages):
        return current_build_out_dir("cargo diagnostic\n" + "\n".join(map(json.dumps, messages)), self.manifest)

    def script(self, out_dir, package=None):
        return {"reason": "build-script-executed", "package_id": package or self.package,
                "out_dir": str(out_dir)}

    def test_selects_current_output_despite_other_profiles_and_same_name_packages(self):
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary)
            outputs = [target / profile / "build" / name / "out"
                       for profile, name in [("release", "freshness_probe-old"),
                                             ("ci-test", "freshness_probe-old"),
                                             ("debug", "freshness_probe-old"),
                                             ("debug", "freshness_probe-current")]]
            for output in outputs:
                output.mkdir(parents=True)
                (output / "runtime-build-info.bin").write_bytes(b"fixture")
            foreign = "path+file:///fixture/other/engine#freshness_probe@0.1.0"
            foreign_artifact = {**self.artifact, "manifest_path": "/fixture/other/engine/Cargo.toml",
                                "package_id": foreign}
            for fresh in [False, True]:
                with self.subTest(fresh=fresh):
                    selected = self.select([self.script(outputs[0], foreign), foreign_artifact,
                                            {**self.artifact, "fresh": fresh}, self.script(outputs[-1])])
                    self.assertEqual(selected, outputs[-1])
                    self.assertTrue((selected / "runtime-build-info.bin").is_file())

    def test_rejects_missing_ambiguous_or_foreign_current_output(self):
        script = self.script("/target/debug/build/freshness_probe-current/out")
        for messages in [[], [script], [self.artifact],
                         [self.artifact, self.artifact, script],
                         [self.artifact, script, script],
                         [self.artifact, self.script("/foreign/out", "foreign-package")],
                         [{**self.artifact, "manifest_path": "/foreign/Cargo.toml"}, script],
                         [self.artifact, {**script, "out_dir": ""}]]:
            with self.subTest(messages=messages), self.assertRaises(ValueError):
                self.select(messages)


@unittest.skipUnless(sys.platform.startswith("linux"), "executable fixtures require Linux")
class BuildFreshness(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        SCRATCH.mkdir(parents=True, exist_ok=True)
        cls.temporary = tempfile.TemporaryDirectory(prefix="suite-", dir=SCRATCH)
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.suite = Path(cls.temporary.name)
        cls.target = cls.suite / "cargo-target"
        manifest = tomllib.loads((ENGINE / "Cargo.toml").read_text())
        cls.dependencies = {}
        for name, spec in manifest["build-dependencies"].items():
            if isinstance(spec, dict) and spec.get("workspace"):
                inherited = manifest["workspace"]["dependencies"][name]
                spec = dict(inherited) if isinstance(inherited, dict) else {"version": inherited}
                spec.update({k: v for k, v in manifest["build-dependencies"][name].items() if k != "workspace"})
            cls.dependencies[name] = spec
        cls.runtime_shadow = manifest["dependencies"]["shadow-rs"]
        cls.script = (ENGINE / "build.rs").read_bytes()
        reference = os.environ.get("REFACT_BUILD_SCRIPT_REF")
        if reference:
            result = subprocess.run(["git", "show", f"{reference}:refact-agent/engine/build.rs"],
                                    cwd=ROOT, capture_output=True, timeout=30, check=True)
            cls.script = result.stdout
        cls.wrapper = cls.suite / "rustc-wrapper"
        cls.wrapper.write_text(f"#!{sys.executable}\n" + '''import json, os, sys
from pathlib import Path
args = sys.argv[2:]
if '--crate-name' in args and args[args.index('--crate-name') + 1] == 'freshness_probe':
    with Path(os.environ['FRESHNESS_RUSTC_LOG']).open('a') as output:
        output.write(json.dumps(args) + '\\n')
os.execv(sys.argv[1], sys.argv[1:])
''')
        cls.wrapper.chmod(0o755)
        capability = execute([str(cls.wrapper), shutil.which("rustc") or "rustc", "--version"], cls.suite)
        if capability.returncode:
            raise unittest.SkipTest("fixture executable capability unavailable: " + capability.stderr)

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="case-", dir=self.suite)
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.engine = self.root / "engine"
        self.gui = self.root / "gui"
        self.log = self.root / "tools.jsonl"
        self.rustc_log = self.root / "rustc.jsonl"
        self.records = []
        self.engine.mkdir()
        self.gui.mkdir()
        self.write(self.engine / "Cargo.toml", '[workspace]\n[package]\nname = "freshness_probe"\nversion = "0.1.0"\nedition = "2021"\n[build-dependencies]\n' + "\n".join(f'{json.dumps(k)} = {toml_value(v)}' for k, v in self.dependencies.items()) + '\n[profile.release]\nopt-level = 0\ndebug = false\n')
        with (self.engine / "Cargo.toml").open("a") as output:
            output.write('\n[dependencies]\nshadow-rs = ' + toml_value(self.runtime_shadow) + '\n[profile.ci-test]\ninherits = "dev"\ndebug = false\n')
        shutil.copyfile(ENGINE / "Cargo.lock", self.engine / "Cargo.lock")
        (self.engine / "build.rs").write_bytes(self.script)
        for relative in ["build_support/runtime_version.rs", "build_support/gui_artifact.rs", "build_support/gui_artifact/trusted_controller.rs", "assets/chat/index.html", "src/daemon/web_picker.html"]:
            target = self.engine / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ENGINE / relative, target)
        self.write(self.engine / "src/main.rs", 'include!(concat!(env!("OUT_DIR"), "/shadow.rs"));\nfn main() { println!("{}", COMMIT_HASH); }\n')
        package = {"name": "freshness-gui", "version": "1.0.0"}
        self.write(self.gui / "package.json", json.dumps(package))
        self.write(self.gui / "package-lock.json", json.dumps({"lockfileVersion": 3, "packages": {"": package}}))
        for relative in ["src/lib/index.ts", "vite.config.ts", "vite.node.config.ts", "tsconfig.json", ".env", "node_modules/typescript/bin/tsc", "node_modules/vite/bin/vite.js"]:
            self.write(self.gui / relative, "{}\n")
        for name in ["typescript", "vite"]:
            self.write(self.gui / f"node_modules/{name}/package.json", '{"version":"1.0.0"}')
        tools = self.root / "bin"
        tools.mkdir()
        fake = f"#!{sys.executable}\n" + '''import json, os, sys
from pathlib import Path
args = sys.argv[1:]
with Path(os.environ['FRESHNESS_TOOL_LOG']).open('a') as output:
    output.write(json.dumps({'tool': Path(sys.argv[0]).name, 'args': args}) + '\\n')
if Path(sys.argv[0]).name == 'npm':
    if args != ['--version']:
        sys.exit('unexpected npm invocation')
    print('10.0.0')
elif args and args[0] == '-e':
    print(json.dumps({'node':'v22.0.0', 'node_abi':'127', 'typescript':'1.0.0', 'vite':'1.0.0'}))
elif args == ['node_modules/typescript/bin/tsc']:
    pass
elif args == ['node_modules/vite/bin/vite.js', 'build']:
    for name in ['index.js', 'index.umd.cjs', 'style.css', 'index.d.ts']:
        target = Path('dist/chat') / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text('fixture ' + name + '\\n')
elif args == ['node_modules/vite/bin/vite.js', 'build', '-c', 'vite.node.config.ts']:
    target = Path('dist/events/index.js')
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text('fixture events\\n')
else:
    sys.exit('unexpected node invocation: ' + repr(args))
'''
        for name in ["node", "npm"]:
            path = tools / name
            path.write_text(fake)
            path.chmod(0o755)
        self.env = {k: v for k, v in os.environ.items() if not (k.startswith(("REFACT_", "VITE_", "CARGO_PROFILE_", "GIT_")) or k in {"NODE_OPTIONS", "NODE_ENV", "GITHUB_SHA", "CI_COMMIT_SHA", "VERCEL_GIT_COMMIT_SHA", "BUILD_VCS_NUMBER", "SOURCE_DATE_EPOCH", "RUSTC_WORKSPACE_WRAPPER", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"})}
        self.env.update(PATH=str(tools) + os.pathsep + os.environ["PATH"], CARGO_TARGET_DIR=str(self.target),
                        CARGO_BUILD_JOBS="2", CARGO_NET_OFFLINE="true", RUSTC_WRAPPER=str(self.wrapper),
                        FRESHNESS_TOOL_LOG=str(self.log), FRESHNESS_RUSTC_LOG=str(self.rustc_log),
                        VITE_FRESHNESS_TEST="one", GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_SYSTEM="/dev/null")
        self.write(self.root / ".gitignore", "engine/target/\ngui/node_modules/\ngui/dist/\nengine/assets/chat/dist/\nengine/assets/.refact-gui-*/\nbin/\n*.jsonl\n")
        self.git("init", "-q")
        self.git("add", "engine", "gui", ".gitignore")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture")

    @staticmethod
    def write(path, text):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def git(self, *args):
        result = execute(["git", *args], self.root, self.env, 30)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def lines(self, path):
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def compilers(self):
        return [entry for entry in self.lines(self.log) if entry["args"] and entry["args"][0].startswith("node_modules/")]

    def snapshot(self):
        paths = list((self.engine / "assets/chat/dist").rglob("*")) + list((self.gui / "dist").rglob("*"))
        paths += list(self.target.glob("debug/build/freshness_probe-*/out/shadow.rs"))
        binary = self.target / "debug" / NAME
        if binary.exists():
            paths.append(binary)
        return ({str(path): path.stat().st_mtime_ns for path in paths if path.is_file()},
                len(list((self.engine / "assets").glob(".refact-gui-*"))),
                len(self.lines(self.log)), len(self.lines(self.rustc_log)))

    def cargo(self, command="build", release=False, success=True, profile=None):
        args = ["cargo", command, "--offline", "--jobs", "2", "--message-format=json"]
        if release:
            args.append("--release")
        if profile:
            args.extend(["--profile", profile])
        result = execute(args, self.engine, self.env)
        if success:
            self.assertEqual(result.returncode, 0, result.stderr[-16000:] + result.stdout[-4000:])
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        artifacts = []
        for line in result.stdout.splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get("reason") == "compiler-artifact" and item["target"]["name"] == NAME and "bin" in item["target"]["kind"]:
                artifacts.append(item)
        if success:
            self.assertEqual(len(artifacts), 1, result.stdout)
        record = {"command": args, "fresh": artifacts[0]["fresh"] if artifacts else None,
                  "compiler_tools": len(self.compilers()), "rustc": len(self.lines(self.rustc_log)),
                  "retained": self.snapshot()[1]}
        self.records.append(record)
        print(json.dumps(record), flush=True)
        return record["fresh"], result

    def warm(self):
        before = self.snapshot()
        self.assertTrue(self.cargo()[0], "unchanged build must report Cargo Fresh")
        self.assertEqual(self.snapshot(), before, "Fresh build must preserve output mtimes, tool calls, rustc calls and retention count")

    def changed(self, action):
        time.sleep(0.03)
        before = len(self.compilers())
        rustc = len(self.lines(self.rustc_log))
        action()
        self.assertFalse(self.cargo()[0])
        self.assertEqual(len(self.compilers()), before + 3)
        self.assertEqual(len(self.lines(self.rustc_log)), rustc + 1)
        self.warm()

    def test_default_warm_build_and_run(self):
        self.assertFalse(self.cargo()[0])
        self.assertEqual(len(self.compilers()), 3)
        self.assertEqual(len(self.lines(self.rustc_log)), 1)
        self.warm()
        before = self.snapshot()
        self.assertTrue(self.cargo("run")[0])
        self.assertEqual(self.snapshot(), before)
        self.warm()
        with self.subTest(profile="ci-test inherits dev with debug=false"):
            self.assertFalse(self.cargo(profile="ci-test")[0])
            before = self.snapshot()
            self.assertTrue(self.cargo(profile="ci-test")[0])
            self.assertEqual(self.snapshot(), before)

    def test_source_optional_lock_environment_and_git_changes(self):
        self.cargo()
        cases = [
            ("source edit", lambda: self.write(self.gui / "src/lib/index.ts", "export const changed = 1;\n")),
            ("new source file", lambda: self.write(self.gui / "src/lib/new.ts", "export {};\n")),
            ("existing optional env edit", lambda: self.write(self.gui / ".env", "VITE_FILE=two\n")),
            ("existing optional env deletion", lambda: (self.gui / ".env").unlink()),
            ("lockfile edit", lambda: self.write(self.gui / "package-lock.json", (self.gui / "package-lock.json").read_text() + "\n")),
            ("existing VITE variable", lambda: self.env.update(VITE_FRESHNESS_TEST="two")),
            ("Git current ref update", lambda: self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-qm", "identity changed")),
        ]
        for name, action in cases:
            with self.subTest(name=name):
                self.changed(action)

    def test_always_build_and_release_keep_strict_revalidation(self):
        for mode in ["always", "release"]:
            with self.subTest(mode=mode):
                if mode == "always":
                    self.env["REFACT_GUI_ALWAYS_BUILD"] = "1"
                else:
                    self.env.pop("REFACT_GUI_ALWAYS_BUILD", None)
                before = len(self.compilers())
                retained = self.snapshot()[1]
                for _ in range(2):
                    self.assertFalse(self.cargo(release=mode == "release")[0])
                self.assertEqual(len(self.compilers()), before + 6)
                self.assertEqual(self.snapshot()[1], retained + 2)

    def test_skip_gui_settles_without_controller_or_gui_inputs(self):
        self.env.update(REFACT_SKIP_GUI_BUILD="1", REFACT_GUI_ALWAYS_BUILD="1", REFACT_USE_PREBUILT_GUI="1")
        shutil.rmtree(self.gui)
        self.git("checkout", "-b", "freshness/nested/identity")
        self.cargo()
        self.assertEqual(self.lines(self.log), [])
        self.warm()
        self.assertTrue(self.cargo("run")[0])
        self.assertEqual(self.lines(self.log), [])
        self.git("pack-refs", "--all", "--prune")
        self.cargo()
        self.warm()
        previous = self.git("rev-parse", "HEAD")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "commit", "--allow-empty", "-qm", "advance packed ref")
        current = self.git("rev-parse", "HEAD")
        self.assertNotEqual(previous, current)
        fresh, result = self.cargo()
        self.assertFalse(fresh, "a new loose ref must invalidate packed-ref identity")
        out_dir = current_build_out_dir(result.stdout, self.engine / "Cargo.toml")
        frame = (out_dir / "runtime-build-info.bin").read_bytes()
        prefix, suffix = b"\0REFACT_BUILD_INFO_V1\0", b"\0END_REFACT_BUILD_INFO\0"
        self.assertTrue(frame.startswith(prefix) and frame.endswith(suffix))
        self.assertEqual(json.loads(frame[len(prefix):-len(suffix)])["source_sha"], current)
        self.warm()
        self.assertEqual(self.lines(self.log), [])

    def test_prebuilt_precedes_always_and_refuses_before_compiler(self):
        if CONTROLLER.exists():
            self.skipTest("negative ingress case requires absent host controller; host controller is never modified")
        self.env.update(REFACT_USE_PREBUILT_GUI="1", REFACT_GUI_ALWAYS_BUILD="1")
        before = self.snapshot()
        for _ in range(2):
            _, result = self.cargo(success=False)
            self.assertIn("GUI prebuilt requested", result.stderr)
            self.assertIn("GUI artifact refusal", result.stderr)
            self.assertTrue("Missing or unsafe admin GUI controller" in result.stderr or "GUI provenance" in result.stderr, result.stderr)
            self.assertEqual(self.lines(self.log), [])
            self.assertEqual(self.lines(self.rustc_log), [])
        self.assertEqual(self.snapshot(), before)


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
"""Hermetic tests for tools/dev/runtime_manifest.py (fake repos and fake release files only)."""
from __future__ import annotations

import copy
import os
import hashlib
import io
import json
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))
import runtime_manifest as rm  # noqa: E402

FIXTURE = REPO / "refact-agent/engine/crates/refact-self-update/tests/fixtures/runtime-manifest-v1.json"
FIXTURE_DIGEST = "5ff571f38849b95bb0d1aa626ebe150a64c72a8ddcf750ea2f93e28bdd7bb9f2"
PACKAGER = REPO / "tools/dev/package_engine_release_assets.sh"
VERSION = "8.7.0-main-12-abcdef12"
INVENTORY = {"schema": 1, "required_jobs": ["engine", "gui"],
             "entries": [{"id": "gui-types", "lane": "gui", "required": True, "kind": "command"},
                         {"id": "engine-reusable", "lane": "engine", "required": True, "kind": "reusable"},
                         {"id": "buddy-live", "lane": "tooling", "required": False, "kind": "command",
                          "unavailable": "needs credentials"}]}
RUN = {"repository_id": 123, "run_id": 789, "run_attempt": 1}


def build_info(target, version=VERSION, sha=None, extra=b""):
    payload = json.dumps({"schema_version": 1, "version": version, "source_sha": sha, "target": target},
                         separators=(",", ":")).encode()
    return b"ELF..." + rm.BUILD_INFO_HEADER + payload + rm.BUILD_INFO_FOOTER + extra + b"...end"


def git(root, *args):
    return subprocess.check_output(["git", *args], cwd=root, text=True).strip()


def make_repo(root: Path) -> str:
    for rel in (*rm.CONFIG_FILES, *rm.JAVASCRIPT_LOCKS, "refact-agent/engine/Cargo.lock",
                ".github/workflows/build_all.yml", ".github/workflows/tests.yml"):
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(f"content of {rel}\n")
    (root / rm.INVENTORY_FILE).write_text(json.dumps(INVENTORY))
    shutil.copy(REPO / rm.COMPATIBILITY_FILE, root / rm.COMPATIBILITY_FILE)
    subprocess.check_call(["git", "init", "-q", "-b", "main", str(root)])
    git(root, "-c", "user.email=t@example.invalid", "-c", "user.name=t", "add", "-A")
    git(root, "-c", "user.email=t@example.invalid", "-c", "user.name=t", "commit", "-q", "-m", "init")
    return git(root, "rev-parse", "HEAD")


def sidecar(path: Path):
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    path.with_name(path.name + ".sha256").write_text(f"{digest}  {path.name}\n")


def zip_bytes(entries: dict) -> bytes:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as zf:
        for name, data in entries.items():
            zf.writestr(name, data)
    return buf.getvalue()


def make_release(out: Path, tmp: Path, sha: str, targets=rm.DEFAULT_RUNTIME_TARGETS, channel="preview"):
    dist = tmp / "dist"
    for t in targets:
        d = dist / f"dist-{t}"
        d.mkdir(parents=True)
        binary = "refact.exe" if "windows" in t else "refact"
        (d / binary).write_bytes(build_info(t, sha=sha))
        (d / ("refact-lsp.exe" if "windows" in t else "refact-lsp")).write_bytes(b"lsp")
    subprocess.check_call(["bash", str(PACKAGER), "--version", VERSION, "--input-dir", str(dist),
                           "--output-dir", str(out)], stdout=subprocess.DEVNULL)
    gui = out / "refact-chat-js-8.7.0.tgz"
    with tarfile.open(gui, "w:gz") as tf:
        data = json.dumps({"name": "refact-chat-js", "version": "8.7.0"}).encode()
        info = tarfile.TarInfo("package/package.json")
        info.size = len(data)
        tf.addfile(info, io.BytesIO(data))
    vsix = out / f"codify-{VERSION}.vsix"
    pre = '<Property Id="Microsoft.VisualStudio.Code.PreRelease" Value="true" />' if channel != "stable" else ""
    vsix.write_bytes(zip_bytes({"extension/package.json": json.dumps(
        {"name": "codify", "publisher": "smallcloud", "version": VERSION.split("-")[0],
         "refactRuntimeVersion": VERSION}),
        "extension.vsixmanifest": f"<PackageManifest><Properties>{pre}</Properties></PackageManifest>"}))
    plugin_xml = f"<idea-plugin><id>{rm.JETBRAINS_PLUGIN_ID}</id><version>{VERSION}</version></idea-plugin>"
    jar = zip_bytes({"META-INF/plugin.xml": plugin_xml})
    jb = out / f"Refact.ai-{VERSION}.zip"
    jb.write_bytes(zip_bytes({"Refact.ai/lib/refact.jar": jar, "Refact.ai/lib/other.jar": zip_bytes({"a": "b"})}))
    for p in (gui, vsix, jb):
        sidecar(p)


def produce_args(root, out, sha, channel="preview", **extra):
    argv = ["produce", "--root", str(root), "--source-sha", sha, "--version", VERSION, "--channel", channel,
            "--rust-toolchain", "1.88.0", "--node-version", "22.23.4", "--profile", "ci-release",
            "--repository-owner", "example", "--repository-name", "refact", "--repository-id", "123",
            "--workflow-path", ".github/workflows/prerelease.yml", "--workflow-id", "456",
            "--run-id", "789", "--run-attempt", "1", "--artifacts-dir", str(out),
            "--output", str(Path(root).parent / "manifest.json")]
    for k, v in extra.items():
        argv += [f"--{k.replace('_', '-')}", v]
    return argv


def publish_step(workflow: str, job: str) -> dict:
    import yaml
    data = yaml.safe_load((REPO / ".github/workflows" / workflow).read_text())
    steps = [st for st in data["jobs"][job]["steps"] if "gh release" in st.get("run", "")]
    assert len(steps) == 1
    return steps[0]


class PublishBlocks(unittest.TestCase):
    """Execute the real publish run blocks with a stub gh; a failing or incomplete verifier must publish nothing."""

    def test_qualification_id_downloads_use_expected_directory(self):
        import yaml
        workflow = yaml.safe_load((REPO / '.github/workflows/build_all.yml').read_text())
        steps = [step for job in workflow['jobs'].values() for step in job.get('steps', [])
                 if 'artifact-ids' in step.get('with', {})]
        self.assertEqual(len(steps), 2)
        for step in steps:
            self.assertTrue(step['with']['merge-multiple'])
            self.assertEqual(step['with']['path'], '${{ runner.temp }}/qualification')

    def test_publication_source_is_data_only_and_trusted_verifier_is_isolated(self):
        import yaml
        workflow = yaml.safe_load((REPO / '.github/workflows/release.yml').read_text())
        self.assertIn('tests', workflow['jobs']['publish-runtime']['needs'])
        steps = workflow['jobs']['publish-runtime']['steps']
        source = next(s for s in steps if s.get('with', {}).get('path') == 'publication-source')
        self.assertEqual(source['uses'], 'actions/checkout@v4')
        self.assertEqual(source['with']['ref'], '${{ needs.resolve.outputs.source_sha }}')
        self.assertIs(source['with']['persist-credentials'], False)
        block = publish_step('release.yml', 'publish-runtime')['run']
        self.assertIn('cd "$(mktemp -d)"', block)
        self.assertIn('python3 -I "$TRUSTED_DIR/tools/dev/runtime_manifest.py" verify-bundle', block)
        self.assertNotIn('"$SOURCE_DIR/tools/', block)
        env = publish_step('release.yml', 'publish-runtime')['env']
        self.assertEqual(env['QUALIFICATION_SHA256'], '${{ needs.tests.outputs.qualification_sha256 }}')
        self.assertEqual(env['QUALIFICATION_ID'], '${{ needs.tests.outputs.qualification_artifact_id }}')
        self.assertEqual(env['REPOSITORY_ID'], '${{ github.repository_id }}')

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.root = self.tmp / "repo"
        self.sha = make_repo(self.root)
        self.dir = self.tmp / "runner" / "release"
        make_release(self.dir, self.tmp, self.sha)
        self.bin = self.tmp / "bin"
        self.bin.mkdir()
        self.log = self.tmp / "gh.log"
        gh = self.bin / "gh"
        gh.write_text('#!/bin/sh\necho "$*" >> "$GH_LOG"\ncase "$*" in\n'
                      '  "api repos/o/r/actions/runs/789 --jq .path") echo ".github/workflows/$WF@refs/heads/main" ;;\n'
                      '  "release view"*) exit 1 ;;\nesac\n')
        gh.chmod(0o755)

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def run_block(self, workflow, job, channel, trusted, manifest_sha=None, qualification_sha=None,
                  qualification_id="42", repository_id="123"):
        manifest = self.tmp / "manifest.json"
        if not (self.dir / rm.MANIFEST_NAME).exists():
            args = produce_args(self.root, self.dir, self.sha, channel)
            args[args.index("--workflow-path") + 1] = f".github/workflows/{workflow}"
            self.assertEqual(rm.main(args), 0)
            shutil.copy(manifest, self.dir / rm.MANIFEST_NAME)
        step = publish_step(workflow, job)
        env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}", GH_LOG=str(self.log), WF=workflow,
                   RUNNER_TEMP=str(self.tmp / "runner"), GITHUB_REPOSITORY="o/r", GITHUB_RUN_ID="789",
                   GITHUB_RUN_ATTEMPT="1", TRUSTED_DIR=str(trusted), SOURCE_SHA=self.sha, VERSION=VERSION,
                   SOURCE_DIR=str(self.root),
                   QUALIFICATION_SHA256=qualification_sha if qualification_sha is not None else getattr(self, 'q_sha', ''),
                   QUALIFICATION_ID=qualification_id, REPOSITORY_ID=repository_id,
                   CHANNEL=channel, MANIFEST_SHA256=manifest_sha or rm.sha256_file(manifest), GH_TOKEN="x")
        return subprocess.run(["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c", step["run"]],
                              env=env, capture_output=True, text=True, cwd=self.tmp)

    def releases(self):
        return [l for l in (self.log.read_text().splitlines() if self.log.exists() else []) if l.startswith("release ")
                and not l.startswith("release view")]

    def failing_trusted(self, body):
        trusted = self.tmp / "trusted"
        (trusted / "tools/dev").mkdir(parents=True, exist_ok=True)
        (trusted / "tools/dev/runtime_manifest.py").write_text(body)
        return trusted

    def test_prerelease_publishes_verified_complete_set(self):
        result = self.run_block("prerelease.yml", "publish", "preview", REPO)
        self.assertEqual(result.returncode, 0, result.stderr)
        [create] = self.releases()
        self.assertTrue(create.startswith(f"release create engine/v{VERSION} "))
        self.assertEqual(create.count(str(self.dir)), 19)
        self.assertIn("--prerelease", create)

    def test_failing_verifier_publishes_nothing(self):
        for body in ("import sys; sys.exit(2)",
                     "import sys, pathlib; a = sys.argv; pathlib.Path(a[a.index('--assets-out') + 1]).write_text('')",
                     "import sys, pathlib; a = sys.argv; pathlib.Path(a[a.index('--assets-out') + 1]).write_text('x\\n'); sys.exit(1)"):
            with self.subTest(body=body):
                self.log.unlink(missing_ok=True)
                result = self.run_block("prerelease.yml", "publish", "preview", self.failing_trusted(body))
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.releases(), [])

    def test_manifest_digest_mismatch_publishes_nothing(self):
        result = self.run_block("prerelease.yml", "publish", "preview", REPO, manifest_sha="0" * 64)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.releases(), [])

    def test_release_runtime_block_rejects_untested_bundle(self):
        result = self.run_block("release.yml", "publish-runtime", "preview", REPO)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.releases(), [])

    def prepare_stable(self):
        shutil.rmtree(self.dir)
        make_release(self.dir, self.tmp / "stable", self.sha, channel="stable")
        q = Produce.qualification(self)
        args = produce_args(self.root, self.dir, self.sha, "stable", qualification=q,
                            qualification_sha256=self.q_sha,
                            qualification_ref=f"github-artifact:123/789/1/42/{self.q_sha}",
                            trusted_inventory=str(self.root / rm.INVENTORY_FILE))
        args[args.index("--workflow-path") + 1] = ".github/workflows/release.yml"
        self.assertEqual(rm.main(args), 0)
        shutil.copy(self.tmp / "manifest.json", self.dir / rm.MANIFEST_NAME)
        shutil.copy(q, self.dir / rm.QUALIFICATION_NAME)
        trusted = self.tmp / "trusted"
        (trusted / "tools/dev").mkdir(parents=True)
        shutil.copy(HERE / "runtime_manifest.py", trusted / "tools/dev/runtime_manifest.py")
        shutil.copy(self.root / rm.INVENTORY_FILE, trusted / rm.INVENTORY_FILE)
        return trusted

    def rewrite_manifest_report_digest(self):
        manifest = self.dir / rm.MANIFEST_NAME
        m = json.loads(manifest.read_bytes())
        m["qualification_ref"] = f"github-artifact:123/789/1/42/{rm.sha256_file(self.dir / rm.QUALIFICATION_NAME)}"
        manifest.write_bytes(rm.canonical_bytes(m))

    def test_stable_publishes_exact_qualification_bytes_and_19_plus_manifest(self):
        trusted = self.prepare_stable()
        expected_report = (self.tmp / "q.json").read_bytes()
        # Source recipes are hashed as data, never imported/executed with the write token.
        # make_repo already populated its runtime_manifest.py with invalid Python content.
        result = self.run_block("release.yml", "publish-runtime", "stable", trusted)
        self.assertEqual(result.returncode, 0, result.stderr)
        [create] = self.releases()
        assets = [a for a in shlex.split(create) if a.startswith(str(self.dir) + "/")]
        self.assertEqual(len(assets), 20)
        self.assertEqual(set(assets), {str(p) for p in self.dir.iterdir()})
        self.assertEqual(assets.count(str(self.dir / rm.QUALIFICATION_NAME)), 1)
        self.assertEqual((self.dir / rm.QUALIFICATION_NAME).read_bytes(), expected_report)
        self.assertNotIn("--prerelease", create)

    def test_stable_bad_public_report_publishes_nothing(self):
        trusted = self.prepare_stable()
        report_path = self.dir / rm.QUALIFICATION_NAME
        original = report_path.read_bytes()
        for case in ("missing", "tampered", "wrong-source", "wrong-inputs", "wrong-run", "missing-required",
                     "oversized", "symlink", "extra-sidecar", "wrong-ref", "duplicate-key", "shrunk-inventory"):
            with self.subTest(case=case):
                self.log.unlink(missing_ok=True)
                report_path.unlink(missing_ok=True)
                report_path.write_bytes(original)
                shutil.copy(self.tmp / "manifest.json", self.dir / rm.MANIFEST_NAME)
                if case == "missing":
                    report_path.unlink()
                elif case == "tampered":
                    report_path.write_bytes(original + b" ")
                elif case == "oversized":
                    report_path.write_bytes(b" " * (rm.MAX_QUALIFICATION_BYTES + 1))
                elif case == "symlink":
                    report_path.unlink()
                    report_path.symlink_to(self.tmp / "q.json")
                elif case == "extra-sidecar":
                    sidecar(report_path)
                elif case == "duplicate-key":
                    report_path.write_bytes(original.replace(b'{', b'{"schema_version":1,', 1))
                    self.rewrite_manifest_report_digest()
                elif case == "wrong-ref":
                    m = json.loads((self.dir / rm.MANIFEST_NAME).read_bytes())
                    m["qualification_ref"] = m["qualification_ref"].replace("123/", "124/", 1)
                    (self.dir / rm.MANIFEST_NAME).write_bytes(rm.canonical_bytes(m))
                else:
                    report = json.loads(original)
                    if case == "wrong-source":
                        report["source_sha"] = "0" * 40
                    elif case == "wrong-inputs":
                        report["build_inputs"]["rust_toolchain"] = "1.89.0"
                    elif case == "wrong-run":
                        report["run_attempt"] = "2"
                    elif case == "shrunk-inventory":
                        inv = copy.deepcopy(INVENTORY)
                        inv["entries"] = [e for e in inv["entries"] if e["id"] != "gui-types"]
                        (self.root / rm.INVENTORY_FILE).write_text(json.dumps(inv))
                        report["inventory_sha256"] = rm.sha256_file(self.root / rm.INVENTORY_FILE)
                        report["results"] = [r for r in report["results"] if r["id"] != "gui-types"]
                    else:
                        report["results"] = report["results"][:-2]
                    report_path.write_text(json.dumps(report))
                    self.rewrite_manifest_report_digest()
                # Deliberately allow the manifest digest check to pass: the real verifier must reject.
                result = self.run_block("release.yml", "publish-runtime", "stable", trusted,
                                        manifest_sha=rm.sha256_file(self.dir / rm.MANIFEST_NAME),
                                        qualification_sha=rm.sha256_file(report_path) if case in
                                        {"wrong-source", "wrong-inputs", "wrong-run", "missing-required",
                                         "duplicate-key", "shrunk-inventory"} else self.q_sha)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertEqual(self.releases(), [])
                self.assertIn({"wrong-ref": "reference identity mismatch", "wrong-source": "source SHA mismatch",
                               "wrong-inputs": "build inputs mismatch", "wrong-run": "run_attempt mismatch",
                               "shrunk-inventory": "drops trusted required coverage", "duplicate-key": "duplicate key",
                               "missing-required": "results do not match inventory", "tampered": "file digest mismatch",
                               "oversized": "byte limit exceeded", "symlink": "regular file",
                               "missing": "regular file", "extra-sidecar": "bundle contents differ"}[case], result.stderr)
                (self.dir / (rm.QUALIFICATION_NAME + ".sha256")).unlink(missing_ok=True)
                (self.root / rm.INVENTORY_FILE).write_text(json.dumps(INVENTORY))

    def test_stable_valid_report_rebound_to_manifest_still_requires_original_tests_bytes(self):
        trusted = self.prepare_stable()
        report = self.dir / rm.QUALIFICATION_NAME
        original_sha = self.q_sha
        report.write_bytes(report.read_bytes() + b" ")
        self.assertNotEqual(rm.sha256_file(report), original_sha)
        # It remains semantically valid and all required results still PASS.
        m = json.loads((self.dir / rm.MANIFEST_NAME).read_bytes())
        rm.verify_qualification(report, self.sha, m['build_inputs'], root=self.root,
                                repository_id=123, run_id=789, run_attempt=1,
                                report_sha256=rm.sha256_file(report), trusted_inventory=trusted / rm.INVENTORY_FILE)
        self.rewrite_manifest_report_digest()
        result = self.run_block('release.yml', 'publish-runtime', 'stable', trusted,
                                manifest_sha=rm.sha256_file(self.dir / rm.MANIFEST_NAME))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('qualification reference differs from Tests receipt', result.stderr)
        self.assertEqual(self.releases(), [])

    def test_stable_verifier_emitting_19_paths_then_failing_publishes_nothing(self):
        trusted = self.prepare_stable()
        body = ("import sys, pathlib; a = sys.argv; "
                "pathlib.Path(a[a.index('--assets-out') + 1]).write_text('x\\n' * 19); sys.exit(7)")
        (trusted / 'tools/dev/runtime_manifest.py').write_text(body)
        result = self.run_block('release.yml', 'publish-runtime', 'stable', trusted)
        self.assertEqual(result.returncode, 7)
        self.assertEqual(len((self.tmp / 'runner/assets.txt').read_text().splitlines()), 19)
        self.assertEqual(self.releases(), [])

    def test_stable_missing_or_mismatched_tests_receipt_publishes_nothing(self):
        trusted = self.prepare_stable()
        for override, reason in [({'qualification_sha': ''}, 'requires Tests qualification sha256'),
                                 ({'qualification_id': ''}, 'requires Tests qualification artifact id'),
                                 ({'qualification_id': '43'}, 'differs from Tests receipt'),
                                 ({'repository_id': '124'}, 'differs from Tests receipt')]:
            with self.subTest(override=override):
                self.log.unlink(missing_ok=True)
                result = self.run_block('release.yml', 'publish-runtime', 'stable', trusted, **override)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(reason, result.stderr)
                self.assertEqual(self.releases(), [])

    def test_preview_report_publishes_nothing(self):
        result = self.run_block("prerelease.yml", "publish", "preview", REPO)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.log.unlink()
        (self.dir / rm.QUALIFICATION_NAME).write_text("{}")
        result = self.run_block("prerelease.yml", "publish", "preview", REPO)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.releases(), [])


class FixtureParity(unittest.TestCase):
    def test_fixture_canonical_digest_matches_rust(self):
        m = json.loads(FIXTURE.read_text())
        self.assertEqual(rm.manifest_digest(m), FIXTURE_DIGEST)
        reordered = copy.deepcopy(m)
        reordered["artifacts"].reverse()
        self.assertEqual(rm.manifest_digest(reordered), FIXTURE_DIGEST)
        self.assertEqual(rm.canonical_bytes(json.loads(rm.canonical_bytes(m))), rm.canonical_bytes(m))

    def test_build_inputs_digest_is_struct_order_compact_json(self):
        bi = json.loads(FIXTURE.read_text())["build_inputs"]
        expected = hashlib.sha256(json.dumps(bi, separators=(",", ":")).encode()).hexdigest()
        self.assertEqual(rm.build_inputs_digest(dict(reversed(list(bi.items())))), expected)

    def test_js_lock_map_digest_is_order_independent_and_bounded(self):
        a = {"refact-agent/gui/package-lock.json": "1" * 64, "plugins/vscode/package-lock.json": "2" * 64}
        first = rm.path_map_digest(a)
        self.assertEqual(first, rm.path_map_digest(dict(reversed(list(a.items())))))
        self.assertEqual(first, hashlib.sha256(json.dumps(dict(sorted(a.items())), separators=(",", ":")).encode()).hexdigest())
        for bad in ({}, {"../package-lock.json": "4" * 64}, {"a": "A" * 64}, {f"p{i}": "1" * 64 for i in range(33)}):
            with self.assertRaises(rm.ManifestError):
                rm.path_map_digest(bad)

    def test_schema_rejections(self):
        base = json.loads(FIXTURE.read_text())
        cases = [
            lambda m: m.update(extra=1),
            lambda m: m["build"].update(channel="beta"),
            lambda m: m.update(source_sha="1" * 39),
            lambda m: m["build_inputs"].update(cargo_lock_sha256="X" * 64),
            lambda m: m["artifacts"][0].update(filename="../x.tar.gz"),
            lambda m: m["artifacts"][0].update(format="zip"),
            lambda m: m["artifacts"][0]["extracted_files"][0].update(path="/abs"),
            lambda m: m["artifacts"].append(dict(m["artifacts"][1])),
            lambda m: m["compatibility"]["daemon_api"].update(min=3),
            lambda m: m.update(artifacts=[a for a in m["artifacts"] if a["component"] != "runtime"]),
            lambda m: m.update(tests="failed"),
            lambda m: m["build"].update(channel="stable"),
            lambda m: m.update(qualification_ref="x"),
            lambda m: m.update(schema_version=2),
        ]
        for i, mutate in enumerate(cases):
            m = copy.deepcopy(base)
            mutate(m)
            with self.subTest(case=i), self.assertRaises(rm.ManifestError):
                rm.canonical_manifest(m)


class BuildInputs(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.root = self.tmp / "repo"
        self.sha = make_repo(self.root)

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def test_hashes_real_files_and_binds_workflows(self):
        bi = rm.compute_build_inputs(self.root, "1.88.0", "22.23.4", "ci-release")
        self.assertEqual(list(bi), list(rm.BUILD_INPUT_FIELDS))
        self.assertEqual(bi["cargo_lock_sha256"], rm.sha256_file(self.root / "refact-agent/engine/Cargo.lock"))
        locks = {p: rm.sha256_file(self.root / p) for p in rm.JAVASCRIPT_LOCKS}
        self.assertEqual(bi["javascript_locks_sha256"], rm.path_map_digest(locks))
        (self.root / ".github/workflows/tests.yml").write_text("changed\n")
        self.assertNotEqual(rm.compute_build_inputs(self.root, "1.88.0", "22.23.4", "ci-release")["config_sha256"],
                            bi["config_sha256"])

    def test_rejects_unsafe_or_floating_inputs_and_missing_files(self):
        for args in (("stable", "22.23.4", "ci-release"), ("1.88.0", "22", "ci-release"),
                     ("1.88.0", "v22.23.4", "ci-release"), ("1.88.0", "22.23.4", "../x"),
                     ("1.88.0;rm", "22.23.4", "ci-release")):
            with self.subTest(args=args), self.assertRaises(rm.ManifestError):
                rm.compute_build_inputs(self.root, *args)
        (self.root / rm.JAVASCRIPT_LOCKS[0]).unlink()
        with self.assertRaises(rm.ManifestError):
            rm.compute_build_inputs(self.root, "1.88.0", "22.23.4", "ci-release")

    def test_cli_build_inputs(self):
        out = subprocess.run([sys.executable, str(HERE / "runtime_manifest.py"), "build-inputs", "--root", str(self.root),
                              "--rust-toolchain", "nightly-2026-11-04", "--node-version", "22.23.4",
                              "--profile", "ci-release"], capture_output=True, text=True, check=True)
        data = json.loads(out.stdout)
        self.assertEqual(set(data), {"build_inputs", "build_inputs_digest"})
        self.assertEqual(data["build_inputs_digest"], rm.build_inputs_digest(data["build_inputs"]))


class Produce(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.root = self.tmp / "repo"
        self.sha = make_repo(self.root)
        self.out = self.tmp / "release"
        make_release(self.out, self.tmp, self.sha)
        self.manifest_path = self.tmp / "manifest.json"

    def tearDown(self):
        shutil.rmtree(self.tmp)

    def run_cli(self, argv):
        return rm.main(argv)

    def qualification(self, **override):
        bi = rm.compute_build_inputs(self.root, "1.88.0", "22.23.4", "ci-release")
        results = [{"id": "engine", "status": "PASS", "evidence_source": "job"}, {"id": "gui", "status": "PASS"},
                   {"id": "gui-types", "status": "PASS"}, {"id": "engine-reusable", "status": "PASS"},
                   {"id": "buddy-live", "status": "EXCLUDED", "reason": "needs credentials"}]
        report = {"schema_version": 1, "kind": "refact-tests-qualification", "source_sha": self.sha,
                  "repository_id": "123", "run_id": "789", "run_attempt": "1",
                  "inventory_sha256": rm.sha256_file(self.root / rm.INVENTORY_FILE), "build_inputs": bi,
                  "build_inputs_digest": rm.build_inputs_digest(bi), "tests": "passed", "status": "PASS",
                  "coverage_complete": True, "results": results}
        report.update(override)
        path = self.tmp / "q.json"
        path.write_text(json.dumps(report))
        self.q_sha = rm.sha256_file(path)
        return str(path)

    def stable(self, q, ref=None, **extra):
        trusted = self.tmp / "trusted-inventory.json"
        if not trusted.exists():
            trusted.write_text(json.dumps(INVENTORY))
        return produce_args(self.root, self.out, self.sha, "stable", qualification=q,
                            qualification_ref=ref or f"github-artifact:123/789/1/42/{self.q_sha}",
                            qualification_sha256=extra.pop("sha", self.q_sha), trusted_inventory=str(trusted), **extra)

    def test_preview_manifest_records_actual_files_and_is_untested(self):
        self.assertEqual(self.run_cli(produce_args(self.root, self.out, self.sha)), 0)
        raw = self.manifest_path.read_bytes()
        m = json.loads(raw)
        self.assertEqual(raw, rm.canonical_bytes(m))
        self.assertEqual(m["tests"], "not_run")
        self.assertNotIn("qualification_ref", m)
        self.assertEqual(m["component_versions"], {"runtime": VERSION, "gui": "8.7.0", "vscode": "8.7.0", "jetbrains": VERSION})
        self.assertEqual(m["compatibility"]["host_bridge"], {"min": 1, "max": 1})
        runtime = [a for a in m["artifacts"] if a["component"] == "runtime"]
        self.assertEqual(sorted(a["target"] for a in runtime), sorted(rm.DEFAULT_RUNTIME_TARGETS))
        for a in m["artifacts"]:
            f = self.out / a["filename"]
            self.assertEqual((a["size"], a["sha256"]), (f.stat().st_size, rm.sha256_file(f)))
        linux = next(a for a in runtime if a["target"] == "x86_64-unknown-linux-gnu")
        data = build_info("x86_64-unknown-linux-gnu", sha=self.sha)
        self.assertEqual(linux["extracted_files"], [{"path": "refact", "size": len(data),
                                                     "sha256": hashlib.sha256(data).hexdigest()}])
        self.assertEqual(rm.build_inputs_digest(m["build_inputs"]),
                         rm.build_inputs_digest(rm.compute_build_inputs(self.root, "1.88.0", "22.23.4", "ci-release")))
        self.assertNotIn("conclusion", json.dumps(m))

    def stable_release(self):
        shutil.rmtree(self.out)
        make_release(self.out, self.tmp / "stable", self.sha, channel="stable")

    def test_stable_requires_matching_complete_qualification(self):
        self.stable_release()
        self.assertEqual(self.run_cli(produce_args(self.root, self.out, self.sha, "stable")), 2)
        results = json.loads(Path(self.qualification()).read_text())["results"]
        bad = [dict(source_sha="0" * 40), dict(tests="blocked"), dict(status="BLOCKED"), dict(coverage_complete=False),
               dict(coverage_complete="true"), dict(coverage_complete=None), dict(build_inputs_digest="0" * 64),
               dict(results=[]), dict(schema_version=2), dict(kind="other"), dict(run_attempt="2"), dict(run_id="790"),
               dict(repository_id="124"), dict(run_attempt=1), dict(inventory_sha256="0" * 64),
               dict(results=results[:-1]), dict(results=results + [{"id": "extra", "status": "PASS"}]),
               dict(results=results + [results[0]]),
               dict(results=[dict(r, status="BLOCKED") if r["id"] == "gui-types" else r for r in results]),
               dict(results=[dict(r, status="EXCLUDED") if r["id"] == "engine-reusable" else r for r in results]),
               dict(results=[dict(r, status="BLOCKED") if r["id"] == "engine" else r for r in results])]
        for override in bad:
            with self.subTest(override=override):
                self.assertEqual(self.run_cli(self.stable(self.qualification(**override))), 2)
        q = self.qualification()
        report = json.loads(Path(q).read_text())
        report["build_inputs"]["rust_toolchain"] = "1.89.0"
        Path(q).write_text(json.dumps(report))
        self.assertEqual(self.run_cli(self.stable(q, sha=rm.sha256_file(Path(q)))), 2)
        q = self.qualification()
        self.assertEqual(self.run_cli(self.stable(q, sha="0" * 64)), 2)
        self.assertEqual(self.run_cli(self.stable(q)), 0)
        m = json.loads(self.manifest_path.read_text())
        self.assertEqual((m["tests"], m["build"]["channel"]), ("passed", "stable"))

    def test_forged_pass_report_cannot_shrink_trusted_inventory(self):
        self.stable_release()
        shrunk = dict(INVENTORY, entries=[e for e in INVENTORY["entries"] if e["id"] != "gui-types"])
        (self.root / rm.INVENTORY_FILE).write_text(json.dumps(shrunk))
        results = [r for r in json.loads(Path(self.qualification()).read_text())["results"] if r["id"] != "gui-types"]
        q = self.qualification(results=results)
        self.assertEqual(self.run_cli(self.stable(q)), 2)

    def test_optional_entry_exclusion_requires_unavailable(self):
        self.stable_release()
        inv = json.loads(json.dumps(INVENTORY))
        del inv["entries"][2]["unavailable"]
        (self.root / rm.INVENTORY_FILE).write_text(json.dumps(inv))
        self.assertEqual(self.run_cli(self.stable(self.qualification())), 2)

    def test_prerelease_cannot_claim_tested(self):
        q = self.qualification()
        self.assertEqual(self.run_cli(produce_args(self.root, self.out, self.sha, "preview", qualification=q,
                                                   qualification_ref="x")), 2)

    def test_wrong_source_sha(self):
        self.assertEqual(self.run_cli(produce_args(self.root, self.out, "f" * 40)), 2)
        self.assertEqual(self.run_cli(produce_args(self.root, self.out, self.sha[:12])), 2)

    def test_missing_or_tampered_artifacts(self):
        def expect_fail(mutate):
            shutil.rmtree(self.out)
            make_release(self.out, self.tmp / f"t{len(list(self.tmp.iterdir()))}", self.sha)
            mutate()
            self.assertEqual(self.run_cli(produce_args(self.root, self.out, self.sha)), 2)

        expect_fail(lambda: (self.out / f"refact-{VERSION}-aarch64-apple-darwin.tar.gz").unlink()
                    or (self.out / f"refact-{VERSION}-aarch64-apple-darwin.tar.gz.sha256").unlink())
        expect_fail(lambda: (self.out / f"codify-{VERSION}.vsix.sha256").unlink())
        expect_fail(lambda: (self.out / f"codify-{VERSION}.vsix").unlink()
                    or (self.out / f"codify-{VERSION}.vsix.sha256").unlink())
        expect_fail(lambda: (self.out / f"Refact.ai-{VERSION}.zip").write_bytes(b"tampered"))
        expect_fail(lambda: (self.out / "notes.txt").write_text("unrelated"))
        expect_fail(lambda: (self.out / f"refact-9.9.9-x86_64-unknown-linux-gnu.tar.gz").write_bytes(b"x"))

    def test_runtime_archive_layout_enforced(self):
        target = "x86_64-unknown-linux-gnu"
        path = self.out / f"refact-{VERSION}-{target}.tar.gz"
        with tarfile.open(path, "w:gz") as tf:
            for name in ("refact", "../evil"):
                info = tarfile.TarInfo(name)
                info.size = 1
                tf.addfile(info, io.BytesIO(b"x"))
        sidecar(path)
        self.assertEqual(self.run_cli(produce_args(self.root, self.out, self.sha)), 2)

    def bundle_args(self, **override):
        values = dict(manifest=str(self.out / rm.MANIFEST_NAME), dir=str(self.out), source_sha=self.sha,
                      channel="preview", workflow_path=".github/workflows/prerelease.yml", version=VERSION,
                      run_id="789", run_attempt="1", assets_out=str(self.tmp / "assets.txt"))
        values.update(override)
        argv = ["verify-bundle"]
        for k, v in values.items():
            argv += [f"--{k.replace('_', '-')}", v]
        return argv

    def test_verify_bundle_gate(self):
        self.assertEqual(self.run_cli(produce_args(self.root, self.out, self.sha)), 0)
        shutil.copy(self.manifest_path, self.out / rm.MANIFEST_NAME)
        self.assertEqual(self.run_cli(self.bundle_args()), 0)
        self.assertEqual(len((self.tmp / "assets.txt").read_text().splitlines()), 18)
        for override in (dict(source_sha="0" * 40), dict(channel="stable"), dict(run_attempt="2"),
                         dict(version="8.7.1"), dict(workflow_path=".github/workflows/release.yml")):
            with self.subTest(override=override):
                self.assertEqual(self.run_cli(self.bundle_args(**override)), 2)
        (self.out / "latest.txt").write_text("unrelated")
        self.assertEqual(self.run_cli(self.bundle_args()), 2)
        (self.out / "latest.txt").unlink()
        (self.out / f"codify-{VERSION}.vsix").write_bytes(b"swapped")
        self.assertEqual(self.run_cli(self.bundle_args()), 2)

    def test_verify_bundle_rejects_incomplete_release_set(self):
        self.assertEqual(self.run_cli(produce_args(self.root, self.out, self.sha)), 0)
        m = json.loads(self.manifest_path.read_text())
        name = next(a["filename"] for a in m["artifacts"] if a["target"] == "aarch64-apple-darwin")
        m["artifacts"] = [a for a in m["artifacts"] if a["filename"] != name]
        (self.out / name).unlink()
        (self.out / f"{name}.sha256").unlink()
        (self.out / rm.MANIFEST_NAME).write_bytes(rm.canonical_bytes(m))
        self.assertEqual(self.run_cli(self.bundle_args()), 2)

    def test_runtime_build_info_must_match(self):
        target = "x86_64-unknown-linux-gnu"
        cases = [build_info(target, version="8.7.0", sha=self.sha), build_info(target, sha="f" * 40),
                 build_info("aarch64-unknown-linux-gnu", sha=self.sha), b"no frame here",
                 build_info(target, sha=self.sha, extra=build_info(target, sha=self.sha)),
                 rm.BUILD_INFO_HEADER + b"{" + rm.BUILD_INFO_FOOTER,
                 build_info(target, sha=self.sha.upper()),
                 build_info(target, sha=self.sha).replace(b'"schema_version":1', b'"schema_version":true'),
                 build_info(target, sha=self.sha).replace(b'{"schema_version":1,', b'{"schema_version":1,"schema_version":1,'),
                 build_info(target, sha=self.sha).replace(b'"source_sha"', b'"extra":1,"source_sha"')]
        for data in cases:
            with self.subTest(data=data[:40]):
                d = self.tmp / "one" / f"dist-{target}"
                shutil.rmtree(d.parent, ignore_errors=True)
                d.mkdir(parents=True)
                (d / "refact").write_bytes(data)
                subprocess.check_call(["bash", str(PACKAGER), "--version", VERSION, "--input-dir", str(d.parent),
                                       "--output-dir", str(self.out)], stdout=subprocess.DEVNULL)
                self.assertEqual(self.run_cli(produce_args(self.root, self.out, self.sha)), 2)
        literal_fragments = rm.BUILD_INFO_HEADER + b"garbage" + build_info(target, sha=self.sha) + rm.BUILD_INFO_FOOTER
        self.assertEqual(rm.parse_build_info(literal_fragments)["target"], target)

    def test_vsix_metadata_must_be_numeric_and_flagged(self):
        vsix = self.out / f"codify-{VERSION}.vsix"
        for version, flagged in (("8.7.0-main-12-abcdef12", True), ("8.7.1", True), ("8.7.0", False)):
            with self.subTest(version=version, flagged=flagged):
                pre = '<Property Id="Microsoft.VisualStudio.Code.PreRelease" Value="true" />' if flagged else ""
                vsix.write_bytes(zip_bytes({"extension/package.json": json.dumps(
                    {"name": "codify", "publisher": "smallcloud", "version": version,
                     "refactRuntimeVersion": VERSION}),
                    "extension.vsixmanifest": f"<PackageManifest>{pre}</PackageManifest>"}))
                sidecar(vsix)
                self.assertEqual(self.run_cli(produce_args(self.root, self.out, self.sha)), 2)

    def test_vsix_runtime_requirement_must_match_exact_full_build_version(self):
        vsix = self.out / f"codify-{VERSION}.vsix"
        for runtime in (None, "", "8.7.0", "8.7.1-main-12-abcdef12", 87):
            with self.subTest(runtime=runtime):
                package = {"name": "codify", "publisher": "smallcloud", "version": VERSION.split("-")[0]}
                if runtime is not None:
                    package["refactRuntimeVersion"] = runtime
                vsix.write_bytes(zip_bytes({"extension/package.json": json.dumps(package),
                    "extension.vsixmanifest": '<PackageManifest><Property Id="Microsoft.VisualStudio.Code.PreRelease" Value="true" /></PackageManifest>'}))
                sidecar(vsix)
                self.assertEqual(self.run_cli(produce_args(self.root, self.out, self.sha)), 2)

    def test_jetbrains_upload_result_fails_closed(self):
        rm.classify_jetbrains_upload(200, b'{"id": 1}')
        for status, body in ((400, b"plugin already contains version"), (401, b""), (500, b"x"), (302, b"")):
            with self.subTest(status=status), self.assertRaises(rm.ManifestError):
                rm.classify_jetbrains_upload(status, body)
        body = self.tmp / "body.txt"
        body.write_text("already contains version")
        self.assertEqual(self.run_cli(["jetbrains-upload-result", "--status", "400", "--body", str(body)]), 2)

    def test_verify_qualification_cli(self):
        self.qualification()
        trusted = self.tmp / "trusted-inventory.json"
        trusted.write_text(json.dumps(INVENTORY))
        base = ["verify-qualification", "--root", str(self.root), "--rust-toolchain", "1.88.0",
                "--node-version", "22.23.4", "--profile", "ci-release", "--trusted-inventory", str(trusted),
                "--repository-id", "123", "--run-id", "789", "--run-attempt", "1"]
        q = self.qualification()
        self.assertEqual(self.run_cli(base + ["--source-sha", self.sha, "--qualification", q, "--qualification-sha256", self.q_sha]), 0)
        q = self.qualification(coverage_complete=False)
        self.assertEqual(self.run_cli(base + ["--source-sha", self.sha, "--qualification", q, "--qualification-sha256", self.q_sha]), 2)
        self.assertEqual(self.run_cli(base + ["--source-sha", self.sha, "--qualification", str(self.tmp / "nope"),
                                              "--qualification-sha256", "0" * 64]), 2)
        q = self.qualification()
        self.assertEqual(self.run_cli(base + ["--source-sha", "e" * 40, "--qualification", q, "--qualification-sha256", self.q_sha]), 2)

    def test_missing_compatibility_file(self):
        (self.root / rm.COMPATIBILITY_FILE).unlink()
        self.assertEqual(self.run_cli(produce_args(self.root, self.out, self.sha)), 2)


if __name__ == "__main__":
    unittest.main()

"use strict";

const assert = require("node:assert/strict");
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { expectedSha256, resolveArtifact, sha256, TARGETS } = require("../postinstall.js");

const installer = path.join(__dirname, "..", "postinstall.js");

test("dry-run resolves every release target without network access", () => {
  for (const [key, [rustTarget, extension]] of Object.entries(TARGETS)) {
    const [platform, arch] = key.split(":");
    const artifact = resolveArtifact({
      platform,
      arch,
      version: "9.8.7",
      releaseBaseUrl: "https://mirror.example/releases/download/",
    });
    assert.match(artifact.url, /^https:\/\/mirror\.example\/releases\/download\/engine\/v9\.8\.7\/refact-9\.8\.7-/);
    assert.equal(artifact.checksumUrl, `${artifact.url}.sha256`);

    const result = spawnSync(process.execPath, [installer, "--dry-run"], {
      encoding: "utf8",
      env: {
        ...process.env,
        REFACT_INSTALL_PLATFORM: platform,
        REFACT_INSTALL_ARCH: arch,
        REFACT_VERSION: "9.8.7",
        REFACT_RELEASE_BASE_URL: "https://offline.example/releases/download",
      },
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(
      result.stdout.trim(),
      `https://offline.example/releases/download/engine/v9.8.7/refact-9.8.7-${rustTarget}.${extension}`,
    );
  }
});

test("unsupported platforms fail before network access", () => {
  assert.throws(
    () => resolveArtifact({ platform: "plan9", arch: "mips", version: "1.0.0" }),
    /unsupported platform: plan9 mips/,
  );
});

test("reads release sidecars and hashes archives", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "refact-ai-test-"));
  try {
    const archive = path.join(directory, "archive");
    const sidecar = path.join(directory, "archive.sha256");
    fs.writeFileSync(archive, "refact archive fixture\n");
    const digest = sha256(archive);
    fs.writeFileSync(sidecar, `${digest.toUpperCase()}  archive\n`);
    assert.equal(expectedSha256(sidecar), digest);
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

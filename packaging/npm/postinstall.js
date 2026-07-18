#!/usr/bin/env node

"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const http = require("node:http");
const https = require("node:https");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { pipeline } = require("node:stream/promises");

const DEFAULT_RELEASE_BASE_URL = "https://github.com/JegernOUTT/refact/releases/download";
const PACKAGE_VERSION = require("./package.json").version;
const MAX_REDIRECTS = 5;

const TARGETS = Object.freeze({
  "darwin:arm64": ["aarch64-apple-darwin", "tar.gz", "refact"],
  "darwin:x64": ["x86_64-apple-darwin", "tar.gz", "refact"],
  "linux:arm64": ["aarch64-unknown-linux-gnu", "tar.gz", "refact"],
  "linux:x64": ["x86_64-unknown-linux-gnu", "tar.gz", "refact"],
  "win32:arm64": ["aarch64-pc-windows-msvc", "zip", "refact.exe"],
  "win32:ia32": ["i686-pc-windows-msvc", "zip", "refact.exe"],
  "win32:x64": ["x86_64-pc-windows-msvc", "zip", "refact.exe"],
});

function resolveArtifact(options = {}) {
  const platform = options.platform || process.env.REFACT_INSTALL_PLATFORM || process.platform;
  const arch = options.arch || process.env.REFACT_INSTALL_ARCH || process.arch;
  const version = options.version || process.env.REFACT_VERSION || PACKAGE_VERSION;
  const releaseBaseUrl = (
    options.releaseBaseUrl ||
    process.env.REFACT_RELEASE_BASE_URL ||
    DEFAULT_RELEASE_BASE_URL
  ).replace(/\/+$/, "");
  const target = TARGETS[`${platform}:${arch}`];

  if (!target) {
    throw new Error(`unsupported platform: ${platform} ${arch}`);
  }

  const [rustTarget, extension, executableName] = target;
  const archiveName = `refact-${version}-${rustTarget}.${extension}`;
  const url = `${releaseBaseUrl}/engine/v${version}/${archiveName}`;
  return {
    platform,
    arch,
    version,
    rustTarget,
    extension,
    executableName,
    archiveName,
    url,
    checksumUrl: `${url}.sha256`,
  };
}

function requestToFile(url, destination, redirects = 0) {
  return new Promise((resolve, reject) => {
    const client = url.startsWith("https:") ? https : http;
    const request = client.get(url, { headers: { "User-Agent": "refact-ai-npm-installer" } }, (response) => {
      const status = response.statusCode || 0;
      if (status >= 300 && status < 400 && response.headers.location) {
        response.resume();
        if (redirects >= MAX_REDIRECTS) {
          reject(new Error(`too many redirects while downloading ${url}`));
          return;
        }
        const redirected = new URL(response.headers.location, url).toString();
        requestToFile(redirected, destination, redirects + 1).then(resolve, reject);
        return;
      }
      if (status !== 200) {
        response.resume();
        reject(new Error(`download returned HTTP ${status} for ${url}`));
        return;
      }

      const output = fs.createWriteStream(destination, { mode: 0o600 });
      pipeline(response, output).then(resolve, reject);
    });
    request.setTimeout(30_000, () => request.destroy(new Error(`download timed out for ${url}`)));
    request.on("error", reject);
  });
}

function sha256(file) {
  const hash = crypto.createHash("sha256");
  hash.update(fs.readFileSync(file));
  return hash.digest("hex");
}

function expectedSha256(checksumFile) {
  const match = fs.readFileSync(checksumFile, "utf8").match(/^([0-9a-fA-F]{64})(?:\s|$)/);
  if (!match) {
    throw new Error("release checksum sidecar does not contain a SHA-256 digest");
  }
  return match[1].toLowerCase();
}

function extractArchive(artifact, archivePath, extractDirectory) {
  const command = artifact.extension === "zip" && process.platform === "win32"
    ? ["powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", "Expand-Archive -LiteralPath $args[0] -DestinationPath $args[1] -Force", archivePath, extractDirectory]]
    : ["tar", [artifact.extension === "zip" ? "-xf" : "-xzf", archivePath, "-C", extractDirectory]];
  const result = spawnSync(command[0], command[1], { stdio: "pipe", encoding: "utf8" });
  if (result.error) {
    throw new Error(`could not extract release archive: ${result.error.message}`);
  }
  if (result.status !== 0) {
    const detail = (result.stderr || result.stdout || "unknown extraction error").trim();
    throw new Error(`could not extract release archive: ${detail}`);
  }
}

async function install(artifact) {
  const temporaryDirectory = fs.mkdtempSync(path.join(os.tmpdir(), "refact-ai-"));
  const archivePath = path.join(temporaryDirectory, artifact.archiveName);
  const checksumPath = `${archivePath}.sha256`;
  const extractDirectory = path.join(temporaryDirectory, "extract");
  const vendorDirectory = path.join(__dirname, "vendor");

  try {
    fs.mkdirSync(extractDirectory);
    await requestToFile(artifact.url, archivePath);
    await requestToFile(artifact.checksumUrl, checksumPath);
    const expected = expectedSha256(checksumPath);
    const actual = sha256(archivePath);
    if (actual !== expected) {
      throw new Error(`SHA-256 mismatch for ${artifact.archiveName}`);
    }
    extractArchive(artifact, archivePath, extractDirectory);
    const extractedBinary = path.join(extractDirectory, artifact.executableName);
    if (!fs.existsSync(extractedBinary)) {
      throw new Error(`release archive did not contain ${artifact.executableName} at its root`);
    }
    fs.mkdirSync(vendorDirectory, { recursive: true });
    const installedBinary = path.join(vendorDirectory, artifact.executableName);
    fs.copyFileSync(extractedBinary, installedBinary);
    if (artifact.platform !== "win32") {
      fs.chmodSync(installedBinary, 0o755);
    }
    console.log(`Installed Refact ${artifact.version} for ${artifact.platform} ${artifact.arch}.`);
  } finally {
    fs.rmSync(temporaryDirectory, { recursive: true, force: true });
  }
}

async function main(argv = process.argv.slice(2)) {
  const artifact = resolveArtifact();
  if (argv.includes("--dry-run")) {
    console.log(artifact.url);
    return;
  }
  await install(artifact);
}

if (require.main === module) {
  main().catch((error) => {
    console.error(`Refact installation failed: ${error.message}`);
    console.error("Check your network or proxy, then retry with `npm rebuild refact-ai`.");
    console.error("For a release mirror, set REFACT_RELEASE_BASE_URL and retry.");
    process.exitCode = 1;
  });
}

module.exports = { DEFAULT_RELEASE_BASE_URL, TARGETS, expectedSha256, resolveArtifact, sha256 };

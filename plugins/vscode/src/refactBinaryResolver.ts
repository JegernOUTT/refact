import { spawn } from "child_process";
import * as crypto from "crypto";
import * as fs from "fs";
import * as http from "http";
import * as https from "https";
import * as os from "os";
import * as path from "path";
import { compareVersions } from "./refactDaemon";

export const REFACT_RELEASE_BASE_URL = "https://github.com/JegernOUTT/refact/releases/download";
const USER_AGENT_HEADER = "User-Agent";

export type RefactReleaseAsset = {
    target: string;
    archiveName: string;
    archiveUrl: string;
    sha256Url: string;
};

export type RefactBinaryResolverOptions = {
    explicitPath?: string;
    minVersion: string;
    pinnedVersion: string;
    cacheDir: string;
    pathEnv?: string;
    homeDir?: string;
    platform?: string;
    arch?: string;
    runVersion?: (binPath: string) => Promise<string | undefined>;
    downloadFile?: (url: string, destPath: string) => Promise<void>;
    extractArchive?: (archivePath: string, destDir: string, platform: string) => Promise<void>;
    chmod?: (binPath: string) => Promise<void>;
};

export function binaryNameForPlatform(platform: string = process.platform): string {
    return platform === "win32" ? "refact.exe" : "refact";
}

export function refactReleaseTarget(platform: string = process.platform, arch: string = process.arch): string {
    if (platform === "win32") {
        if (arch === "x64") { return "x86_64-pc-windows-msvc"; }
        if (arch === "ia32") { return "i686-pc-windows-msvc"; }
        if (arch === "arm64") { return "aarch64-pc-windows-msvc"; }
    }
    if (platform === "linux") {
        if (arch === "x64") { return "x86_64-unknown-linux-gnu"; }
        if (arch === "arm64") { return "aarch64-unknown-linux-gnu"; }
    }
    if (platform === "darwin") {
        if (arch === "x64") { return "x86_64-apple-darwin"; }
        if (arch === "arm64") { return "aarch64-apple-darwin"; }
    }
    throw new Error(`unsupported Refact release target for ${platform}/${arch}`);
}

export function refactReleaseAsset(version: string, target: string, platform: string = process.platform): RefactReleaseAsset {
    const extension = platform === "win32" ? "zip" : "tar.gz";
    const archiveName = `refact-${version}-${target}.${extension}`;
    const archiveUrl = `${REFACT_RELEASE_BASE_URL}/engine/v${version}/${archiveName}`;
    return {
        target,
        archiveName,
        archiveUrl,
        sha256Url: `${archiveUrl}.sha256`,
    };
}

export function extractRefactVersion(output: string | undefined): string | undefined {
    const text = output?.trim();
    if (!text) { return undefined; }
    const refactMatch = text.match(/(?:^|\s)refact\s+([0-9]+(?:\.[0-9]+){0,2}(?:[-+][0-9A-Za-z._-]+)?)/i);
    if (refactMatch?.[1]) { return refactMatch[1]; }
    const genericMatch = text.match(/([0-9]+(?:\.[0-9]+){1,2}(?:[-+][0-9A-Za-z._-]+)?)/);
    return genericMatch?.[1];
}

export async function resolveRefactBinary(options: RefactBinaryResolverOptions): Promise<string> {
    const explicitPath = options.explicitPath?.trim();
    if (explicitPath) {
        return path.resolve(explicitPath);
    }

    const minVersion = options.minVersion;
    const platform = options.platform ?? process.platform;
    const arch = options.arch ?? process.arch;
    const runVersion = options.runVersion ?? readRefactVersion;
    for (const candidate of systemRefactCandidates(options.pathEnv ?? process.env.PATH ?? "", options.homeDir ?? os.homedir(), platform)) {
        if (await isCompatibleRefactBinary(candidate, minVersion, runVersion)) {
            return candidate;
        }
    }

    return downloadPinnedRefactBinary({ ...options, platform, arch, runVersion });
}

function systemRefactCandidates(pathEnv: string, homeDir: string, platform: string): string[] {
    const binaryName = binaryNameForPlatform(platform);
    const candidates = pathEnv
        .split(path.delimiter)
        .filter(entry => entry.trim().length > 0)
        .map(entry => path.join(entry, binaryName));
    candidates.push(path.join(homeDir, ".refact", "bin", binaryName));
    return Array.from(new Set(candidates.map(candidate => path.resolve(candidate))));
}

async function isCompatibleRefactBinary(
    binPath: string,
    minVersion: string,
    runVersion: (binPath: string) => Promise<string | undefined>,
): Promise<boolean> {
    if (!fileExists(binPath)) { return false; }
    const version = extractRefactVersion(await runVersion(binPath));
    return !!version && compareVersions(version, minVersion) >= 0;
}

async function downloadPinnedRefactBinary(options: Required<Pick<RefactBinaryResolverOptions, "platform" | "arch" | "runVersion">> & RefactBinaryResolverOptions): Promise<string> {
    const target = refactReleaseTarget(options.platform, options.arch);
    const binaryName = binaryNameForPlatform(options.platform);
    const targetDir = path.join(options.cacheDir, options.pinnedVersion, target);
    const binPath = path.join(targetDir, binaryName);
    if (await isCompatibleRefactBinary(binPath, options.minVersion, options.runVersion)) {
        return binPath;
    }

    const downloadFile = options.downloadFile ?? defaultDownloadFile;
    const extractArchive = options.extractArchive ?? defaultExtractArchive;
    const chmod = options.chmod ?? defaultChmod;
    const asset = refactReleaseAsset(options.pinnedVersion, target, options.platform);
    const tmpDir = path.join(options.cacheDir, `tmp-${process.pid}-${Date.now()}-${Math.random().toString(16).slice(2)}`);
    const archivePath = path.join(tmpDir, asset.archiveName);
    const shaPath = `${archivePath}.sha256`;
    const extractDir = path.join(tmpDir, "extract");
    await fs.promises.mkdir(extractDir, { recursive: true });
    try {
        await downloadFile(asset.archiveUrl, archivePath);
        await downloadFile(asset.sha256Url, shaPath);
        await verifySha256(archivePath, shaPath);
        await extractArchive(archivePath, extractDir, options.platform);
        const extractedBin = path.join(extractDir, binaryName);
        if (!fileExists(extractedBin)) {
            throw new Error(`downloaded Refact archive did not contain ${binaryName}`);
        }
        await chmod(extractedBin);
        await fs.promises.rm(targetDir, { recursive: true, force: true });
        await fs.promises.mkdir(path.dirname(targetDir), { recursive: true });
        await fs.promises.rename(extractDir, targetDir);
        await chmod(binPath);
        if (!await isCompatibleRefactBinary(binPath, options.minVersion, options.runVersion)) {
            throw new Error(`downloaded Refact binary is older than ${options.minVersion}`);
        }
        return binPath;
    } finally {
        await fs.promises.rm(tmpDir, { recursive: true, force: true }).catch(() => undefined);
    }
}

async function readRefactVersion(binPath: string): Promise<string | undefined> {
    return runAndCapture(binPath, ["--version"], 5000);
}

function fileExists(candidate: string): boolean {
    try {
        return fs.statSync(candidate).isFile();
    } catch {
        return false;
    }
}

async function verifySha256(archivePath: string, shaPath: string): Promise<void> {
    const expected = expectedSha256(await fs.promises.readFile(shaPath, "utf8"));
    const actual = await sha256File(archivePath);
    if (actual !== expected) {
        throw new Error(`sha256 mismatch for ${path.basename(archivePath)}`);
    }
}

function expectedSha256(text: string): string {
    const match = text.match(/[a-fA-F0-9]{64}/);
    if (!match) {
        throw new Error("sha256 sidecar did not contain a checksum");
    }
    return match[0].toLowerCase();
}

function sha256File(filePath: string): Promise<string> {
    return new Promise((resolve, reject) => {
        const hash = crypto.createHash("sha256");
        const stream = fs.createReadStream(filePath);
        stream.on("data", chunk => hash.update(chunk));
        stream.once("error", reject);
        stream.once("end", () => resolve(hash.digest("hex")));
    });
}

function defaultDownloadFile(url: string, destPath: string): Promise<void> {
    return new Promise((resolve, reject) => {
        downloadFileToPath(url, destPath, 0, resolve, reject);
    });
}

function downloadFileToPath(
    url: string,
    destPath: string,
    redirects: number,
    resolve: () => void,
    reject: (error: Error) => void,
): void {
    const client = url.startsWith("http:") ? http : https;
    const request = client.get(url, { headers: { [USER_AGENT_HEADER]: "refact-vscode" } }, response => {
        const statusCode = response.statusCode ?? 0;
        const location = response.headers.location;
        if (statusCode >= 300 && statusCode < 400 && location && redirects < 5) {
            response.resume();
            downloadFileToPath(new URL(location, url).toString(), destPath, redirects + 1, resolve, reject);
            return;
        }
        if (statusCode !== 200) {
            response.resume();
            reject(new Error(`download failed ${statusCode} ${url}`));
            return;
        }
        fs.mkdirSync(path.dirname(destPath), { recursive: true });
        const output = fs.createWriteStream(destPath);
        response.pipe(output);
        output.once("finish", () => output.close(resolve));
        output.once("error", error => reject(error));
    });
    request.once("error", error => reject(error));
}

async function defaultExtractArchive(archivePath: string, destDir: string, platform: string): Promise<void> {
    try {
        await runProcess("tar", archivePath.endsWith(".zip") ? ["-xf", archivePath, "-C", destDir] : ["-xzf", archivePath, "-C", destDir]);
    } catch (error) {
        if (platform !== "win32" || !archivePath.endsWith(".zip")) {
            throw error;
        }
        await runProcess("powershell.exe", [
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "$ErrorActionPreference='Stop'; Expand-Archive -LiteralPath $args[0] -DestinationPath $args[1] -Force",
            archivePath,
            destDir,
        ]);
    }
}

async function defaultChmod(binPath: string): Promise<void> {
    if (process.platform !== "win32") {
        await fs.promises.chmod(binPath, 0o755);
    }
}

function runAndCapture(command: string, args: string[], timeoutMs: number): Promise<string | undefined> {
    return new Promise(resolve => {
        const child = spawn(command, args, { stdio: ["ignore", "pipe", "pipe"] });
        const chunks: Buffer[] = [];
        let timedOut = false;
        const timer = setTimeout(() => {
            timedOut = true;
            child.kill();
        }, timeoutMs);
        child.stdout?.on("data", chunk => chunks.push(Buffer.from(chunk)));
        child.stderr?.on("data", chunk => chunks.push(Buffer.from(chunk)));
        child.once("error", () => {
            clearTimeout(timer);
            resolve(undefined);
        });
        child.once("close", code => {
            clearTimeout(timer);
            if (timedOut || code !== 0) {
                resolve(undefined);
                return;
            }
            resolve(Buffer.concat(chunks).toString("utf8"));
        });
    });
}

function runProcess(command: string, args: string[]): Promise<void> {
    return new Promise((resolve, reject) => {
        const child = spawn(command, args, { stdio: ["ignore", "ignore", "pipe"] });
        const stderr: Buffer[] = [];
        child.stderr?.on("data", chunk => stderr.push(Buffer.from(chunk)));
        child.once("error", reject);
        child.once("close", code => {
            if (code === 0) {
                resolve();
                return;
            }
            reject(new Error(`${command} exited with ${code}: ${Buffer.concat(stderr).toString("utf8")}`));
        });
    });
}

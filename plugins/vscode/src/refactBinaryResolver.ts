import { spawn } from "child_process";
import * as crypto from "crypto";
import * as fs from "fs";
import * as http from "http";
import * as https from "https";
import * as os from "os";
import * as path from "path";
import * as zlib from "zlib";
import { compareVersions, resolveBundledRefactPath } from "./refactDaemon";

export const REFACT_RELEASE_BASE_URL = "https://github.com/JegernOUTT/refact/releases/download";
const USER_AGENT_HEADER = "User-Agent";
const INSTALL_LOCK_NAME = ".install.lock";
const INSTALL_LOCK_RETRY_MS = 100;
const INSTALL_LOCK_TIMEOUT_MS = 120000;
export const INSTALL_LOCK_STALE_MS = 15 * 60000;

export type RefactReleaseAsset = {
    target: string;
    archiveName: string;
    archiveUrl: string;
    sha256Url: string;
};

export type RefactBinaryResolverOptions = {
    explicitPath?: string;
    bundledDir?: string;
    minVersion: string;
    pinnedVersion: string;
    cacheDir: string;
    pathEnv?: string;
    homeDir?: string;
    platform?: string;
    arch?: string;
    runVersion?: (binPath: string) => Promise<string | undefined>;
    onDownloadStart?: () => void;
    downloadFile?: (url: string, destPath: string) => Promise<void>;
    extractArchive?: (archivePath: string, destDir: string, platform: string) => Promise<void>;
    chmod?: (binPath: string) => Promise<void>;
    installLockRetryMs?: number;
    installLockTimeoutMs?: number;
    installLockStaleMs?: number;
    installLockNowMs?: () => number;
};

type DownloadRefactOptions = RefactBinaryResolverOptions & Required<Pick<RefactBinaryResolverOptions, "platform" | "arch" | "homeDir" | "runVersion">>;

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
    const homeDir = options.homeDir ?? os.homedir();
    const runVersion = options.runVersion ?? readRefactVersion;
    const bundledDir = options.bundledDir?.trim();
    if (bundledDir) {
        const bundledPath = resolveBundledRefactPath(bundledDir);
        if (await isCompatibleRefactBinary(bundledPath, minVersion, runVersion)) {
            return path.resolve(bundledPath);
        }
    }
    for (const candidate of systemRefactCandidates(options.pathEnv ?? process.env.PATH ?? "", homeDir, platform)) {
        if (await isCompatibleRefactBinary(candidate, minVersion, runVersion)) {
            return candidate;
        }
    }

    try {
        return await downloadPinnedRefactBinary({ ...options, platform, arch, homeDir, runVersion });
    } catch (error) {
        throw new Error(refactBinaryResolutionFailureMessage(options.pinnedVersion, error));
    }
}

function refactBinaryResolutionFailureMessage(version: string, error: unknown): string {
    return `Refact engine release ${version} is unavailable or failed to download. Set refactai.binaryPath to a compatible local refact binary. ${errorMessage(error)}`;
}

function errorMessage(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
}

function systemRefactCandidates(pathEnv: string, homeDir: string, platform: string): string[] {
    const binaryName = binaryNameForPlatform(platform);
    const candidates = [sharedRefactBinaryPath(homeDir, platform)];
    candidates.push(...pathEnv
        .split(path.delimiter)
        .filter(entry => entry.trim().length > 0)
        .map(entry => path.join(entry, binaryName)));
    return Array.from(new Set(candidates.map(candidate => path.resolve(candidate))));
}

function sharedRefactBinaryPath(homeDir: string, platform: string): string {
    return path.resolve(path.join(homeDir, ".refact", "bin", binaryNameForPlatform(platform)));
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

async function downloadPinnedRefactBinary(options: DownloadRefactOptions): Promise<string> {
    const target = refactReleaseTarget(options.platform, options.arch);
    const binaryName = binaryNameForPlatform(options.platform);
    const binPath = sharedRefactBinaryPath(options.homeDir, options.platform);
    if (await isCompatibleRefactBinary(binPath, options.minVersion, options.runVersion)) {
        return binPath;
    }

    const lockPath = path.join(path.dirname(binPath), INSTALL_LOCK_NAME);
    return withInstallLock(
        lockPath,
        options.installLockRetryMs ?? INSTALL_LOCK_RETRY_MS,
        options.installLockTimeoutMs ?? INSTALL_LOCK_TIMEOUT_MS,
        options.installLockStaleMs ?? INSTALL_LOCK_STALE_MS,
        options.installLockNowMs ?? Date.now,
        async () => {
            if (await isCompatibleRefactBinary(binPath, options.minVersion, options.runVersion)) {
                return binPath;
            }

            const downloadFile = options.downloadFile ?? defaultDownloadFile;
            const extractArchive = options.extractArchive ?? defaultExtractArchive;
            const chmod = options.chmod ?? (candidate => defaultChmod(candidate, options.platform));
            const asset = refactReleaseAsset(options.pinnedVersion, target, options.platform);
            const tmpDir = path.join(options.cacheDir, options.pinnedVersion, target, `tmp-${process.pid}-${Date.now()}-${Math.random().toString(16).slice(2)}`);
            const archivePath = path.join(tmpDir, asset.archiveName);
            const shaPath = `${archivePath}.sha256`;
            const extractDir = path.join(tmpDir, "extract");
            await fs.promises.mkdir(extractDir, { recursive: true });
            try {
                options.onDownloadStart?.();
                await downloadFile(asset.archiveUrl, archivePath);
                await downloadFile(asset.sha256Url, shaPath);
                await verifySha256(archivePath, shaPath);
                await extractArchive(archivePath, extractDir, options.platform);
                const extractedBin = path.join(extractDir, binaryName);
                if (!fileExists(extractedBin)) {
                    throw new Error(`downloaded Refact archive did not contain ${binaryName}`);
                }
                await chmod(extractedBin);
                await promoteBinaryToSharedInstall(extractedBin, binPath, chmod);
                await chmod(binPath);
                if (!await isCompatibleRefactBinary(binPath, options.minVersion, options.runVersion)) {
                    throw new Error(`downloaded Refact binary is older than ${options.minVersion}`);
                }
                return binPath;
            } finally {
                await fs.promises.rm(tmpDir, { recursive: true, force: true }).catch(() => undefined);
            }
        },
    );
}

async function withInstallLock<T>(
    lockPath: string,
    retryMs: number,
    timeoutMs: number,
    staleMs: number,
    nowMs: () => number,
    body: () => Promise<T>,
): Promise<T> {
    await fs.promises.mkdir(path.dirname(lockPath), { recursive: true });
    const handle = await acquireInstallLock(lockPath, Math.max(10, retryMs), Math.max(10, timeoutMs), Math.max(10, staleMs), nowMs);
    const lockText = await writeInstallLockMetadata(handle, nowMs());
    try {
        return await body();
    } finally {
        await handle.close().catch(() => undefined);
        await releaseInstallLock(lockPath, lockText);
    }
}

async function acquireInstallLock(
    lockPath: string,
    retryMs: number,
    timeoutMs: number,
    staleMs: number,
    nowMs: () => number,
): Promise<fs.promises.FileHandle> {
    const startedAt = nowMs();
    while (true) {
        try {
            return await fs.promises.open(lockPath, "wx");
        } catch (error) {
            if ((error as NodeJS.ErrnoException).code !== "EEXIST") {
                throw error;
            }
            if (await breakStaleInstallLock(lockPath, staleMs, nowMs())) {
                continue;
            }
            const elapsedMs = nowMs() - startedAt;
            if (elapsedMs >= timeoutMs) {
                throw new Error(`timed out waiting for Refact install lock at ${lockPath}`);
            }
            await sleep(Math.min(retryMs, Math.max(10, timeoutMs - elapsedMs)));
        }
    }
}

type InstallLockMetadata = {
    pid?: number;
    timestampMs?: number;
};

async function writeInstallLockMetadata(handle: fs.promises.FileHandle, nowMs: number): Promise<string> {
    const lockText = `pid=${process.pid}\ntimestamp_ms=${nowMs}\n`;
    await handle.writeFile(lockText, "utf8");
    await handle.sync();
    return lockText;
}

async function releaseInstallLock(lockPath: string, lockText: string): Promise<void> {
    try {
        if (await fileTextEquals(lockPath, lockText)) {
            await fs.promises.rm(lockPath, { force: true });
        }
    } catch {
        return;
    }
}

async function breakStaleInstallLock(lockPath: string, staleMs: number, nowMs: number): Promise<boolean> {
    const lockText = await readFileIfExists(lockPath);
    const metadata = lockText ? parseInstallLockMetadata(lockText) : undefined;
    const stale = metadata
        ? metadataIsStale(metadata, staleMs, nowMs, lockPath)
        : lockFileIsOlderThan(lockPath, nowMs, staleMs);
    if (!stale) {
        return false;
    }
    if (lockText !== undefined && !await fileTextEquals(lockPath, lockText)) {
        return false;
    }
    return fs.promises.rm(lockPath, { force: true })
        .then(() => true)
        .catch(() => false);
}

function metadataIsStale(metadata: InstallLockMetadata, staleMs: number, nowMs: number, lockPath: string): boolean {
    if (metadata.timestampMs !== undefined && nowMs - metadata.timestampMs >= staleMs) {
        return true;
    }
    if (metadata.pid !== undefined && !lockProcessIsAlive(metadata.pid)) {
        return true;
    }
    return metadata.timestampMs === undefined && lockFileIsOlderThan(lockPath, nowMs, staleMs);
}

function parseInstallLockMetadata(text: string): InstallLockMetadata | undefined {
    const lines = text.split(/\r?\n/).map(line => line.trim()).filter(line => line.length > 0);
    if (lines.length === 0) {
        return undefined;
    }
    let pid: number | undefined;
    let timestampMs: number | undefined;
    for (const line of lines) {
        const separator = line.indexOf("=");
        if (separator <= 0) {
            continue;
        }
        const key = line.slice(0, separator).trim().toLowerCase();
        const value = line.slice(separator + 1).trim();
        if (key === "pid" || key === "owner_pid") {
            pid = parseLockNumber(value);
        } else if (key === "timestamp" || key === "timestamp_ms" || key === "created_at" || key === "created_at_ms") {
            timestampMs = parseLockTimestampMs(value);
        }
    }
    pid = pid ?? parseLockNumber(lines[0]);
    timestampMs = timestampMs ?? parseLockTimestampMs(lines[1]);
    return pid !== undefined || timestampMs !== undefined ? { pid, timestampMs } : undefined;
}

function parseLockNumber(value: string | undefined): number | undefined {
    if (!value || !/^-?\d+$/.test(value)) {
        return undefined;
    }
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : undefined;
}

function parseLockTimestampMs(value: string | undefined): number | undefined {
    const numeric = parseLockNumber(value);
    if (numeric !== undefined) {
        return numeric;
    }
    const parsed = Date.parse(value ?? "");
    return Number.isFinite(parsed) ? parsed : undefined;
}

function lockProcessIsAlive(pid: number): boolean {
    if (!Number.isFinite(pid) || pid <= 0) {
        return false;
    }
    try {
        process.kill(pid, 0);
        return true;
    } catch (error) {
        return (error as NodeJS.ErrnoException).code === "EPERM";
    }
}

function lockFileIsOlderThan(lockPath: string, nowMs: number, staleMs: number): boolean {
    try {
        return nowMs - fs.statSync(lockPath).mtimeMs >= staleMs;
    } catch {
        return false;
    }
}

async function readFileIfExists(filePath: string): Promise<string | undefined> {
    try {
        return await fs.promises.readFile(filePath, "utf8");
    } catch {
        return undefined;
    }
}

async function fileTextEquals(filePath: string, expected: string): Promise<boolean> {
    return await readFileIfExists(filePath) === expected;
}

function sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
}

async function promoteBinaryToSharedInstall(
    extractedBin: string,
    binPath: string,
    chmod: (binPath: string) => Promise<void>,
): Promise<void> {
    const installDir = path.dirname(binPath);
    const tempTarget = path.join(installDir, `.${path.basename(binPath)}.tmp-${process.pid}-${Date.now()}-${Math.random().toString(16).slice(2)}`);
    await fs.promises.mkdir(installDir, { recursive: true });
    try {
        await fs.promises.copyFile(extractedBin, tempTarget);
        await chmod(tempTarget);
        await fs.promises.rename(tempTarget, binPath);
    } finally {
        await fs.promises.rm(tempTarget, { force: true }).catch(() => undefined);
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

async function defaultExtractArchive(archivePath: string, destDir: string, _platform: string): Promise<void> {
    await extractRefactArchive(archivePath, destDir);
}

export async function extractRefactArchive(archivePath: string, destDir: string): Promise<void> {
    await fs.promises.mkdir(destDir, { recursive: true });
    if (archivePath.endsWith(".zip")) {
        await extractZipArchive(archivePath, destDir);
        return;
    }
    await extractTarGzArchive(archivePath, destDir);
}

async function extractTarGzArchive(archivePath: string, destDir: string): Promise<void> {
    const archive = zlib.gunzipSync(await fs.promises.readFile(archivePath));
    const entries = tarEntries(archive, destDir);
    for (const entry of entries) {
        if (entry.type === "directory") {
            await fs.promises.mkdir(path.dirname(entry.targetPath), { recursive: true });
            await assertRealPathInside(path.dirname(entry.targetPath), destDir);
            await assertNotSymlink(entry.targetPath);
            await fs.promises.mkdir(entry.targetPath, { recursive: true });
            await assertRealPathInside(entry.targetPath, destDir);
            continue;
        }
        await fs.promises.mkdir(path.dirname(entry.targetPath), { recursive: true });
        await assertRealPathInside(path.dirname(entry.targetPath), destDir);
        await assertNotSymlink(entry.targetPath);
        await fs.promises.writeFile(entry.targetPath, archive.subarray(entry.dataStart, entry.dataStart + entry.size));
        await assertRealPathInside(entry.targetPath, destDir);
    }
}

type TarEntry = {
    type: "file" | "directory";
    targetPath: string;
    dataStart: number;
    size: number;
};

function tarEntries(archive: Buffer, destDir: string): TarEntry[] {
    const entries: TarEntry[] = [];
    let offset = 0;
    while (offset + 512 <= archive.length) {
        const header = archive.subarray(offset, offset + 512);
        if (header.every(byte => byte === 0)) {
            break;
        }
        const name = tarString(header, 0, 100);
        const prefix = tarString(header, 345, 155);
        const entryName = prefix ? `${prefix}/${name}` : name;
        const size = parseTarOctal(header, 124, 12);
        const typeFlag = String.fromCharCode(header[156] || 48);
        const dataStart = offset + 512;
        const targetPath = safeArchiveEntryTarget(destDir, entryName);
        if (typeFlag === "0" || typeFlag === "\0") {
            entries.push({ type: "file", targetPath, dataStart, size });
        } else if (typeFlag === "5") {
            entries.push({ type: "directory", targetPath, dataStart, size: 0 });
        } else if (typeFlag === "x" || typeFlag === "g") {
        } else {
            throw new Error(`unsupported tar entry type ${typeFlag} for ${entryName}`);
        }
        offset = dataStart + size + tarPadding(size);
    }
    return entries;
}

function tarString(buffer: Buffer, start: number, length: number): string {
    const end = buffer.indexOf(0, start);
    const realEnd = end >= start && end < start + length ? end : start + length;
    return buffer.subarray(start, realEnd).toString("utf8").trim();
}

function parseTarOctal(buffer: Buffer, start: number, length: number): number {
    const value = tarString(buffer, start, length).trim();
    return value ? Number.parseInt(value, 8) : 0;
}

function tarPadding(size: number): number {
    return (512 - (size % 512)) % 512;
}

type ZipEntry = {
    name: string;
    targetPath: string;
    compressedSize: number;
    compressionMethod: number;
    localHeaderOffset: number;
    isDirectory: boolean;
};

async function extractZipArchive(archivePath: string, destDir: string): Promise<void> {
    const archive = await fs.promises.readFile(archivePath);
    const entries = zipEntries(archive, destDir);
    for (const entry of entries) {
        if (entry.isDirectory) {
            await fs.promises.mkdir(path.dirname(entry.targetPath), { recursive: true });
            await assertRealPathInside(path.dirname(entry.targetPath), destDir);
            await assertNotSymlink(entry.targetPath);
            await fs.promises.mkdir(entry.targetPath, { recursive: true });
            await assertRealPathInside(entry.targetPath, destDir);
            continue;
        }
        await fs.promises.mkdir(path.dirname(entry.targetPath), { recursive: true });
        await assertRealPathInside(path.dirname(entry.targetPath), destDir);
        await assertNotSymlink(entry.targetPath);
        await fs.promises.writeFile(entry.targetPath, zipEntryData(archive, entry));
        await assertRealPathInside(entry.targetPath, destDir);
    }
}

function zipEntries(archive: Buffer, destDir: string): ZipEntry[] {
    const eocdOffset = findEndOfCentralDirectory(archive);
    const centralDirectorySize = archive.readUInt32LE(eocdOffset + 12);
    const centralDirectoryOffset = archive.readUInt32LE(eocdOffset + 16);
    const entries: ZipEntry[] = [];
    let offset = centralDirectoryOffset;
    const end = centralDirectoryOffset + centralDirectorySize;
    while (offset < end) {
        if (archive.readUInt32LE(offset) !== 0x02014b50) {
            throw new Error("invalid zip central directory");
        }
        const flags = archive.readUInt16LE(offset + 8);
        const compressionMethod = archive.readUInt16LE(offset + 10);
        const compressedSize = archive.readUInt32LE(offset + 20);
        const nameLength = archive.readUInt16LE(offset + 28);
        const extraLength = archive.readUInt16LE(offset + 30);
        const commentLength = archive.readUInt16LE(offset + 32);
        const externalAttributes = archive.readUInt32LE(offset + 38);
        const localHeaderOffset = archive.readUInt32LE(offset + 42);
        const name = archive.subarray(offset + 46, offset + 46 + nameLength).toString((flags & 0x800) !== 0 ? "utf8" : "binary");
        if ((flags & 1) !== 0) {
            throw new Error(`encrypted zip entry is not supported: ${name}`);
        }
        if (compressionMethod !== 0 && compressionMethod !== 8) {
            throw new Error(`unsupported zip compression method ${compressionMethod} for ${name}`);
        }
        if (((externalAttributes >>> 16) & 0o170000) === 0o120000) {
            throw new Error(`zip entry is a symlink: ${name}`);
        }
        entries.push({
            name,
            targetPath: safeArchiveEntryTarget(destDir, name),
            compressedSize,
            compressionMethod,
            localHeaderOffset,
            isDirectory: name.endsWith("/"),
        });
        offset += 46 + nameLength + extraLength + commentLength;
    }
    return entries;
}

function findEndOfCentralDirectory(archive: Buffer): number {
    const minimumOffset = Math.max(0, archive.length - 65557);
    for (let offset = archive.length - 22; offset >= minimumOffset; offset--) {
        if (archive.readUInt32LE(offset) === 0x06054b50) {
            return offset;
        }
    }
    throw new Error("zip archive is missing end of central directory");
}

function zipEntryData(archive: Buffer, entry: ZipEntry): Buffer {
    const offset = entry.localHeaderOffset;
    if (archive.readUInt32LE(offset) !== 0x04034b50) {
        throw new Error(`invalid zip local header for ${entry.name}`);
    }
    const nameLength = archive.readUInt16LE(offset + 26);
    const extraLength = archive.readUInt16LE(offset + 28);
    const dataStart = offset + 30 + nameLength + extraLength;
    const compressed = archive.subarray(dataStart, dataStart + entry.compressedSize);
    return entry.compressionMethod === 0 ? compressed : zlib.inflateRawSync(compressed);
}

function safeArchiveEntryTarget(destDir: string, entryName: string): string {
    if (!entryName || path.isAbsolute(entryName) || path.win32.isAbsolute(entryName) || path.posix.isAbsolute(entryName) || /^[A-Za-z]:/.test(entryName)) {
        throw new Error(`archive entry escapes target directory: ${entryName}`);
    }
    const parts = entryName.split(/[\\/]+/).filter(part => part.length > 0 && part !== ".");
    if (parts.length === 0 || parts.some(part => part === "..")) {
        throw new Error(`archive entry escapes target directory: ${entryName}`);
    }
    const root = path.resolve(destDir);
    const target = path.resolve(root, ...parts);
    if (target !== root && !target.startsWith(root + path.sep)) {
        throw new Error(`archive entry escapes target directory: ${entryName}`);
    }
    return target;
}

async function assertRealPathInside(targetPath: string, destDir: string): Promise<void> {
    const root = await fs.promises.realpath(destDir);
    const realTarget = await fs.promises.realpath(targetPath);
    if (realTarget !== root && !realTarget.startsWith(root + path.sep)) {
        throw new Error(`archive entry escapes target directory: ${targetPath}`);
    }
}

async function assertNotSymlink(targetPath: string): Promise<void> {
    try {
        if ((await fs.promises.lstat(targetPath)).isSymbolicLink()) {
            throw new Error(`archive entry targets a symlink: ${targetPath}`);
        }
    } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") {
            throw error;
        }
    }
}

async function defaultChmod(binPath: string, platform: string = process.platform): Promise<void> {
    if (platform !== "win32") {
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

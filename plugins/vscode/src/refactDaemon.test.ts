import * as assert from "assert";
import * as crypto from "crypto";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import {
    browserProjectUrl,
    compareVersions,
    daemonOpenProjectUrl,
    daemonSpawnCommand,
    daemonStatusUrl,
    ensureBundledRefactPath,
    ensureDaemon,
    isPluginNewerThanDaemon,
    missingBundledRefactError,
    projectProxyBaseUrl,
    resolveBundledRefactPath,
    type DaemonStatus,
} from "./refactDaemon";
import {
    extractRefactArchive,
    extractRefactVersion,
    refactReleaseAsset,
    resolveRefactBinary,
} from "./refactBinaryResolver";

export async function runRefactDaemonTests() {
    assert.strictEqual(
        daemonStatusUrl(8488),
        "http://127.0.0.1:8488/daemon/v1/status",
    );
    assert.strictEqual(
        daemonOpenProjectUrl(8488),
        "http://127.0.0.1:8488/daemon/v1/projects/open",
    );
    assert.strictEqual(
        projectProxyBaseUrl(8488, "abc/123"),
        "http://127.0.0.1:8488/p/abc%2F123/",
    );
    assert.strictEqual(
        browserProjectUrl("machine.local", 8488, "abc"),
        "http://machine.local:8488/p/abc/",
    );

    assert.strictEqual(compareVersions("8.2.0", "8.1.9"), 1);
    assert.strictEqual(compareVersions("8.1.0", "8.1.0-alpha.1"), 1);
    assert.strictEqual(compareVersions("8.1.0-alpha.2", "8.1.0-alpha.10"), -1);
    assert.strictEqual(compareVersions("8.1.0-alpha.1", "8.1.0-beta.1"), -1);
    assert.strictEqual(compareVersions("8.1.0", "8.1.1"), -1);
    assert.strictEqual(isPluginNewerThanDaemon("8.2.0", "8.1.9"), true);
    assert.strictEqual(isPluginNewerThanDaemon("8.1.0", "8.1.0-alpha.1"), true);
    assert.strictEqual(isPluginNewerThanDaemon("8.1.0", "8.1.0"), false);

    await runBundledRefactSpawnTests();
    await runStandaloneResolutionTests();
    await runArchiveTraversalRejectedTest();
    await runDaemonUpgradeWaitsForExitTest();
    await runDaemonUpgradeTimeoutTest();
}

async function runBundledRefactSpawnTests() {
    const assetPath = fs.mkdtempSync(path.join(os.tmpdir(), "refact-daemon-test-"));
    try {
        const refactPath = resolveBundledRefactPath(assetPath);
        fs.writeFileSync(refactPath, "");

        assert.strictEqual(ensureBundledRefactPath(assetPath), refactPath);

        const spawned: string[] = [];
        let spawnedDaemon = false;
        const status = await spawnAndReturnStatus(refactPath, spawned, () => spawnedDaemon = true);

        await ensureDaemon(refactPath, {
            timeoutMs: 1,
            spawnDaemon: binPath => {
                spawned.push(binPath);
                spawnedDaemon = true;
            },
            readDaemonInfo: async () => spawnedDaemon ? status : undefined,
            sleep: async () => undefined,
        });

        assert.deepStrictEqual(spawned, [refactPath]);

        fs.unlinkSync(refactPath);
        let readAttempts = 0;
        let ensureError: Error | undefined;
        try {
            await ensureDaemon(refactPath, {
                timeoutMs: 1,
                spawnDaemon: binPath => spawned.push(binPath),
                readDaemonInfo: async () => {
                    readAttempts++;
                    return undefined;
                },
                sleep: async () => undefined,
            });
        } catch (error) {
            ensureError = error instanceof Error ? error : new Error(String(error));
        }

        assert.strictEqual(ensureError?.message, `refact binary not found at ${refactPath}`);
        assert.strictEqual(readAttempts, 0);
        assert.deepStrictEqual(spawned, [refactPath]);

        assert.strictEqual(missingBundledRefactError(assetPath), `refact binary not found in ${assetPath} — reinstall the extension`);
    } finally {
        fs.rmSync(assetPath, { recursive: true, force: true });
    }
}

async function runStandaloneResolutionTests() {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "refact-binary-resolver-test-"));
    try {
        const explicit = path.join(root, "custom", process.platform === "win32" ? "refact.exe" : "refact");
        assert.strictEqual(await resolveRefactBinary({
            explicitPath: explicit,
            minVersion: "9.0.0",
            pinnedVersion: "9.0.0",
            cacheDir: path.join(root, "cache"),
            pathEnv: "",
            homeDir: path.join(root, "home"),
            runVersion: async () => undefined,
        }), path.resolve(explicit));

        const binaryName = "refact";
        const pathDir = path.join(root, "path-bin");
        const homeDir = path.join(root, "home");
        const cacheDir = path.join(root, "cache");
        const pathRefact = path.join(pathDir, binaryName);
        const homeRefact = path.join(homeDir, ".refact", "bin", binaryName);
        fs.mkdirSync(path.dirname(pathRefact), { recursive: true });
        fs.mkdirSync(path.dirname(homeRefact), { recursive: true });
        fs.writeFileSync(pathRefact, "refact 8.1.0");
        fs.writeFileSync(homeRefact, "refact 8.1.0");

        const versionChecks: string[] = [];
        const sharedPreferred = await resolveRefactBinary({
            minVersion: "8.1.0",
            pinnedVersion: "8.1.0",
            cacheDir,
            pathEnv: pathDir,
            homeDir,
            platform: "linux",
            arch: "x64",
            runVersion: async binPath => {
                versionChecks.push(binPath);
                return "refact 8.1.0";
            },
        });
        assert.strictEqual(sharedPreferred, homeRefact);
        assert.deepStrictEqual(versionChecks, [homeRefact]);

        fs.writeFileSync(homeRefact, "refact 7.9.0");
        const incompatibleSharedSkipped = await resolveRefactBinary({
            minVersion: "8.1.0",
            pinnedVersion: "8.1.0",
            cacheDir,
            pathEnv: pathDir,
            homeDir,
            platform: "linux",
            arch: "x64",
            runVersion: async binPath => fs.readFileSync(binPath, "utf8"),
        });
        assert.strictEqual(incompatibleSharedSkipped, pathRefact);

        const downloads: string[] = [];
        fs.writeFileSync(pathRefact, "refact 7.9.0");
        const extracted = await resolveRefactBinary({
            minVersion: "8.1.0",
            pinnedVersion: "8.1.0",
            cacheDir,
            pathEnv: pathDir,
            homeDir,
            platform: "linux",
            arch: "x64",
            runVersion: async binPath => fs.readFileSync(binPath, "utf8"),
            downloadFile: async (url, destPath) => {
                downloads.push(url);
                fs.mkdirSync(path.dirname(destPath), { recursive: true });
                if (url.endsWith(".sha256")) {
                    fs.writeFileSync(destPath, `${sha256FileSync(path.join(path.dirname(destPath), "refact-8.1.0-x86_64-unknown-linux-gnu.tar.gz"))}  archive\n`);
                } else {
                    fs.writeFileSync(destPath, "archive");
                }
            },
            extractArchive: async (_archivePath, destDir) => {
                fs.writeFileSync(path.join(destDir, "refact"), "refact 8.1.0");
            },
            chmod: async () => undefined,
        });
        assert.strictEqual(extracted, homeRefact);
        assert.notStrictEqual(extracted, path.join(cacheDir, "8.1.0", "x86_64-unknown-linux-gnu", "refact"));
        assert.deepStrictEqual(downloads, [
            "https://github.com/JegernOUTT/refact/releases/download/engine/v8.1.0/refact-8.1.0-x86_64-unknown-linux-gnu.tar.gz",
            "https://github.com/JegernOUTT/refact/releases/download/engine/v8.1.0/refact-8.1.0-x86_64-unknown-linux-gnu.tar.gz.sha256",
        ]);

        const lockHomeDir = path.join(root, "lock-home");
        const lockCacheDir = path.join(root, "lock-cache");
        const lockRefact = path.join(lockHomeDir, ".refact", "bin", binaryName);
        const lockPath = path.join(path.dirname(lockRefact), ".install.lock");
        fs.mkdirSync(path.dirname(lockRefact), { recursive: true });
        fs.writeFileSync(lockRefact, "refact 7.9.0");
        fs.writeFileSync(lockPath, "held");
        let markPreLockChecksDone: (() => void) | undefined;
        const preLockChecksDone = new Promise<void>(resolve => { markPreLockChecksDone = resolve; });
        let lockDownloads = 0;
        let lockVersionReads = 0;
        const lockedResolve = resolveRefactBinary({
            minVersion: "8.1.0",
            pinnedVersion: "8.1.0",
            cacheDir: lockCacheDir,
            pathEnv: "",
            homeDir: lockHomeDir,
            platform: "linux",
            arch: "x64",
            installLockRetryMs: 5,
            installLockTimeoutMs: 2000,
            runVersion: async binPath => {
                assert.strictEqual(binPath, lockRefact);
                lockVersionReads++;
                if (lockVersionReads === 2) {
                    markPreLockChecksDone?.();
                }
                return fs.readFileSync(binPath, "utf8");
            },
            downloadFile: async () => {
                lockDownloads++;
                throw new Error("download should be skipped after install lock re-check");
            },
            extractArchive: async () => undefined,
            chmod: async () => undefined,
        });
        await preLockChecksDone;
        fs.writeFileSync(lockRefact, "refact 8.1.0");
        fs.rmSync(lockPath, { force: true });
        assert.strictEqual(await lockedResolve, lockRefact);
        assert.strictEqual(lockDownloads, 0);
        assert.strictEqual(lockVersionReads >= 3, true);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }

    assert.strictEqual(extractRefactVersion("refact 8.1.0\n"), "8.1.0");
    assert.deepStrictEqual(refactReleaseAsset("8.1.0", "aarch64-pc-windows-msvc", "win32"), {
        target: "aarch64-pc-windows-msvc",
        archiveName: "refact-8.1.0-aarch64-pc-windows-msvc.zip",
        archiveUrl: "https://github.com/JegernOUTT/refact/releases/download/engine/v8.1.0/refact-8.1.0-aarch64-pc-windows-msvc.zip",
        sha256Url: "https://github.com/JegernOUTT/refact/releases/download/engine/v8.1.0/refact-8.1.0-aarch64-pc-windows-msvc.zip.sha256",
    });
}

function sha256FileSync(filePath: string): string {
    return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

async function runArchiveTraversalRejectedTest() {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "refact-archive-slip-test-"));
    try {
        const archivePath = path.join(root, "evil.zip");
        const destDir = path.join(root, "dest");
        fs.mkdirSync(destDir);
        fs.writeFileSync(archivePath, zipStoredEntry("../evil", Buffer.from("oops")));

        let error: Error | undefined;
        try {
            await extractRefactArchive(archivePath, destDir);
        } catch (caught) {
            error = caught instanceof Error ? caught : new Error(String(caught));
        }

        assert.strictEqual(error?.message, "archive entry escapes target directory: ../evil");
        assert.strictEqual(fs.existsSync(path.join(root, "evil")), false);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
}

function zipStoredEntry(name: string, data: Buffer): Buffer {
    const nameBytes = Buffer.from(name, "utf8");
    const crc = crc32(data);
    const local = Buffer.alloc(30 + nameBytes.length);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x800, 6);
    local.writeUInt16LE(0, 8);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nameBytes.length, 26);
    nameBytes.copy(local, 30);

    const central = Buffer.alloc(46 + nameBytes.length);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(0x800, 8);
    central.writeUInt16LE(0, 10);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(data.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(nameBytes.length, 28);
    nameBytes.copy(central, 46);

    const eocd = Buffer.alloc(22);
    eocd.writeUInt32LE(0x06054b50, 0);
    eocd.writeUInt16LE(1, 8);
    eocd.writeUInt16LE(1, 10);
    eocd.writeUInt32LE(central.length, 12);
    eocd.writeUInt32LE(local.length + data.length, 16);

    return Buffer.concat([local, data, central, eocd]);
}

function crc32(data: Buffer): number {
    let crc = 0xffffffff;
    for (const byte of data) {
        crc ^= byte;
        for (let i = 0; i < 8; i++) {
            crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
        }
    }
    return (crc ^ 0xffffffff) >>> 0;
}

async function spawnAndReturnStatus(
    refactPath: string,
    spawned: string[],
    onSpawn: () => void,
): Promise<DaemonStatus> {
    return ensureDaemon(refactPath, {
        timeoutMs: 1,
        spawnDaemon: binPath => {
            spawned.push(binPath);
            onSpawn();
        },
        readDaemonInfo: async () => spawned.length > 0 ? daemonStatus() : undefined,
        sleep: async () => undefined,
    });
}

async function runDaemonUpgradeWaitsForExitTest() {
    const assetPath = fs.mkdtempSync(path.join(os.tmpdir(), "refact-daemon-upgrade-wait-"));
    try {
        const refactPath = resolveBundledRefactPath(assetPath);
        fs.writeFileSync(refactPath, "");

        let shutdownRequested = false;
        let oldStatusReadsAfterShutdown = 0;
        let processRunningChecks = 0;
        let spawned = false;
        let time = 0;
        const sleeps: number[] = [];

        const status = await ensureDaemon(refactPath, {
            pluginVersion: "8.2.0",
            shutdownTimeoutMs: 1000,
            shutdownPollMs: 200,
            spawnDaemon: binPath => {
                assert.strictEqual(binPath, refactPath);
                assert.strictEqual(shutdownRequested, true);
                assert.strictEqual(oldStatusReadsAfterShutdown >= 3, true);
                spawned = true;
            },
            shutdownDaemon: async (port, reason) => {
                assert.strictEqual(port, 8488);
                assert.strictEqual(reason, "upgrade");
                shutdownRequested = true;
            },
            readDaemonInfo: async () => {
                if (!shutdownRequested) {
                    return daemonStatus("8.1.0");
                }
                if (!spawned) {
                    oldStatusReadsAfterShutdown++;
                    return oldStatusReadsAfterShutdown < 3 ? daemonStatus("8.1.0") : undefined;
                }
                return daemonStatus("8.2.0");
            },
            isProcessRunning: () => {
                processRunningChecks++;
                return processRunningChecks < 4;
            },
            sleep: async ms => {
                sleeps.push(ms);
                time += ms;
            },
            now: () => time,
        });

        assert.strictEqual(status.version, "8.2.0");
        assert.strictEqual(spawned, true);
        assert.strictEqual(oldStatusReadsAfterShutdown >= 3, true);
        assert.deepStrictEqual(sleeps, [200, 200, 200]);
    } finally {
        fs.rmSync(assetPath, { recursive: true, force: true });
    }
}

async function runDaemonUpgradeTimeoutTest() {
    const assetPath = fs.mkdtempSync(path.join(os.tmpdir(), "refact-daemon-upgrade-timeout-"));
    try {
        const refactPath = resolveBundledRefactPath(assetPath);
        fs.writeFileSync(refactPath, "");

        let time = 0;
        let spawnCount = 0;
        let shutdownCount = 0;
        let ensureError: Error | undefined;

        try {
            await ensureDaemon(refactPath, {
                pluginVersion: "8.2.0",
                shutdownTimeoutMs: 600,
                shutdownPollMs: 200,
                spawnDaemon: () => spawnCount++,
                shutdownDaemon: async () => {
                    shutdownCount++;
                },
                readDaemonInfo: async () => daemonStatus("8.1.0"),
                isProcessRunning: () => true,
                sleep: async ms => {
                    time += ms;
                },
                now: () => time,
            });
        } catch (error) {
            ensureError = error instanceof Error ? error : new Error(String(error));
        }

        assert.strictEqual(spawnCount, 0);
        assert.strictEqual(shutdownCount, 1);
        assert.strictEqual(
            ensureError?.message,
            "Refact daemon on port 8488 did not exit within 1s after shutdown. Retry shortly, or stop process 1 before retrying.",
        );

        const command = daemonSpawnCommand(refactPath);
        assert.deepStrictEqual(command, { command: refactPath, args: ["daemon"] });
        assert.notStrictEqual(command.command, path.join(assetPath, process.platform === "win32" ? "refact-lsp.exe" : "refact-lsp"));
    } finally {
        fs.rmSync(assetPath, { recursive: true, force: true });
    }
}

function daemonStatus(version = "8.1.0"): DaemonStatus {
    const status = {} as DaemonStatus;
    status.pid = 1;
    status.version = version;
    status.port = 8488;
    status.started_at_ms = 0;
    status.uptime_secs = 0;
    status.workers = 0;
    return status;
}

runRefactDaemonTests().catch(error => {
    console.error(error);
    process.exitCode = 1;
});

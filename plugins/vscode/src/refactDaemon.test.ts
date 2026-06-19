import * as assert from "assert";
import * as crypto from "crypto";
import * as fs from "fs";
import * as http from "http";
import type { AddressInfo, Socket } from "net";
import * as os from "os";
import * as path from "path";
import {
    DAEMON_OPEN_PROJECT_TIMEOUT_MS,
    browserProjectUrl,
    compareVersions,
    daemonEndpoints,
    daemonOpenProjectUrl,
    daemonSpawnCommand,
    daemonStatusUrl,
    ensureBundledRefactPath,
    ensureDaemon,
    findExistingDaemon,
    isPluginNewerThanDaemon,
    missingBundledRefactError,
    openProject,
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
import {
    backendConfigForStatus,
    effectiveLspPortForStatus,
    shouldReadCapsForCompletion,
} from "./backendStatus";

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
    assert.strictEqual(compareVersions("8.10", "8.2"), -1);
    assert.strictEqual(compareVersions("v8.1.0+build.1", "8.1.0"), 0);
    assert.strictEqual(isPluginNewerThanDaemon("8.2.0", "8.1.9"), true);
    assert.strictEqual(isPluginNewerThanDaemon("8.1.0", "8.1.0-alpha.1"), true);
    assert.strictEqual(isPluginNewerThanDaemon("8.1.0", "8.1.0"), false);
    assert.strictEqual(DAEMON_OPEN_PROJECT_TIMEOUT_MS >= 130000, true);

    runBackendStatusTests();

    await runBundledRefactSpawnTests();
    await runStandaloneResolutionTests();
    await runArchiveTraversalRejectedTest();
    await runDaemonUpgradeWaitsForExitTest();
    await runDaemonUpgradeShutdownFailureTest();
    await runDaemonUpgradeTimeoutTest();
    await runCompatibleDaemonSkipsMissingBinaryTest();
    await runDaemonJsonDiscoveryTest();
    await runDaemonReportedPortFallbackTest(0);
    await runDaemonReportedPortFallbackTest(-1);
    await runDaemonAuthHeaderTest();
    await runOpenProjectStaleTokenRetryTest();
    await runShutdownTokenRotationTest();
    await runDiskPortTokenMismatchRecoveryTest();
    await runSpawnFailureSurfacesLogTest();
}

function runBackendStatusTests() {
    assert.deepStrictEqual(backendConfigForStatus("connecting"), {
        backendReady: false,
        connectionStatus: "connecting",
    });
    assert.deepStrictEqual(backendConfigForStatus("ready"), {
        backendReady: true,
        connectionStatus: "ready",
    });
    assert.strictEqual(effectiveLspPortForStatus(8001, "connecting"), 0);
    assert.strictEqual(effectiveLspPortForStatus(8001, "ready"), 8001);
    assert.strictEqual(shouldReadCapsForCompletion(false, "connecting"), false);
    assert.strictEqual(shouldReadCapsForCompletion(false, "ready"), true);
    assert.strictEqual(shouldReadCapsForCompletion(true, "ready"), false);
}

async function runBundledRefactSpawnTests() {
    const assetPath = fs.mkdtempSync(path.join(os.tmpdir(), "refact-daemon-test-"));
    try {
        const refactPath = resolveBundledRefactPath(assetPath);
        writeExecutable(refactPath, "");

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
                spawnDaemon: binPath => { spawned.push(binPath); },
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
        assert.strictEqual(readAttempts, 1);
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
        writeExecutable(refactPath, "");

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
        writeExecutable(refactPath, "");

        let time = 0;
        let spawnCount = 0;
        let shutdownCount = 0;
        let ensureError: Error | undefined;

        try {
            await ensureDaemon(refactPath, {
                pluginVersion: "8.2.0",
                shutdownTimeoutMs: 600,
                shutdownPollMs: 200,
                spawnDaemon: () => { spawnCount++; },
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

async function runDaemonUpgradeShutdownFailureTest() {
    const assetPath = fs.mkdtempSync(path.join(os.tmpdir(), "refact-daemon-upgrade-shutdown-fail-"));
    try {
        const refactPath = resolveBundledRefactPath(assetPath);
        writeExecutable(refactPath, "");
        let spawnCount = 0;
        let ensureError: Error | undefined;

        try {
            await ensureDaemon(refactPath, {
                pluginVersion: "8.2.0",
                spawnDaemon: () => { spawnCount++; },
                shutdownDaemon: async () => {
                    throw new Error("401 stale token");
                },
                readDaemonInfo: async () => daemonStatus("8.1.0"),
                isProcessRunning: () => false,
            });
        } catch (error) {
            ensureError = error instanceof Error ? error : new Error(String(error));
        }

        assert.strictEqual(spawnCount, 0);
        assert.strictEqual(
            ensureError?.message,
            "Refact daemon shutdown failed before upgrade on port 8488: 401 stale token",
        );
    } finally {
        fs.rmSync(assetPath, { recursive: true, force: true });
    }
}

async function runCompatibleDaemonSkipsMissingBinaryTest() {
    const missingPath = path.join(os.tmpdir(), `missing-refact-${Date.now()}`);
    let reads = 0;
    let spawned = false;
    const status = await ensureDaemon(missingPath, {
        pluginVersion: "8.1.0",
        spawnDaemon: () => {
            spawned = true;
        },
        readDaemonInfo: async (port, authToken) => {
            reads++;
            assert.strictEqual(port, 8488);
            assert.strictEqual(authToken, undefined);
            return daemonStatus("8.1.0");
        },
    });

    assert.strictEqual(status.version, "8.1.0");
    assert.strictEqual(reads, 1);
    assert.strictEqual(spawned, false);
}

async function runDaemonJsonDiscoveryTest() {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "refact-daemon-json-"));
    try {
        const daemonJsonPath = path.join(root, "daemon.json");
        fs.writeFileSync(daemonJsonPath, daemonJson(9234, "disk-token"));

        assert.deepStrictEqual(daemonEndpoints({ port: 8488, daemonJsonPath }), [
            { port: 8488 },
            { port: 9234, authToken: "disk-token" },
        ]);

        const probes: Array<[number, string | undefined]> = [];
        const status = await findExistingDaemon({
            port: 8488,
            pluginVersion: "8.1.0",
            daemonJsonPath,
            readDaemonInfo: async (port, authToken) => {
                probes.push([port, authToken]);
                return port === 9234 ? daemonStatus("8.1.0", port) : undefined;
            },
        });

        assert.strictEqual(status?.port, 9234);
        assert.strictEqual(status?.authToken, "disk-token");
        assert.deepStrictEqual(probes, [[8488, undefined], [9234, "disk-token"]]);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
}

async function runDaemonReportedPortFallbackTest(reportedPort: number) {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "refact-daemon-reported-port-"));
    const sockets = new Set<Socket>();
    const requests: Array<{ method: string; url: string; authorization?: string }> = [];
    const authToken = `reported-port-token-${reportedPort}`;
    let spawned = false;
    const server = http.createServer((request, response) => {
        requests.push({
            method: request.method ?? "",
            url: request.url ?? "",
            authorization: request.headers.authorization,
        });
        response.setHeader("Connection", "close");
        response.setHeader("Content-Type", "application/json");
        if (request.url === "/daemon/v1/status") {
            const version = spawned ? "8.2.0" : "8.1.0";
            response.end(JSON.stringify(daemonStatus(version, reportedPort)));
            return;
        }
        if (request.headers.authorization !== `Bearer ${authToken}`) {
            response.statusCode = 401;
            response.end(JSON.stringify({ error: "missing auth" }));
            return;
        }
        if (request.url === "/daemon/v1/projects/open") {
            response.end(openProjectResponse(`project-reported-port-${reportedPort}`, root));
            return;
        }
        if (request.url === "/daemon/v1/shutdown") {
            response.end(JSON.stringify({ ok: true }));
            return;
        }
        response.statusCode = 404;
        response.end(JSON.stringify({ error: "not found" }));
    });
    server.on("connection", socket => {
        sockets.add(socket);
        socket.once("close", () => sockets.delete(socket));
    });
    server.keepAliveTimeout = 1;
    server.headersTimeout = 1000;
    await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
    try {
        const port = (server.address() as AddressInfo).port;
        const daemonJsonPath = path.join(root, "daemon.json");
        fs.writeFileSync(daemonJsonPath, daemonJson(port, authToken));

        const status = await findExistingDaemon({ port, pluginVersion: "8.1.0", daemonJsonPath });
        assert.strictEqual(status?.port, port);
        assert.strictEqual(status?.authToken, authToken);

        const opened = await openProject(root, { port: status?.port, authToken: status?.authToken, daemonJsonPath });
        assert.strictEqual(opened["project_id"], `project-reported-port-${reportedPort}`);
        const refactPath = path.join(root, "refact");
        writeExecutable(refactPath, "");
        const ensured = await ensureDaemon(refactPath, {
            port,
            pluginVersion: "8.1.0",
            daemonJsonPath,
            spawnDaemon: () => { spawned = true; },
        });
        assert.strictEqual(ensured.port, port);
        assert.strictEqual(spawned, false);

        const upgraded = await ensureDaemon(refactPath, {
            port,
            pluginVersion: "8.2.0",
            daemonJsonPath,
            timeoutMs: 1000,
            shutdownTimeoutMs: 1000,
            shutdownPollMs: 1,
            spawnDaemon: () => { spawned = true; },
            isProcessRunning: () => false,
        });
        assert.strictEqual(upgraded.port, port);
        assert.strictEqual(upgraded.version, "8.2.0");
        assert.deepStrictEqual(requests.map(request => request.authorization), [
            undefined,
            `Bearer ${authToken}`,
            undefined,
            undefined,
            `Bearer ${authToken}`,
            undefined,
            undefined,
        ]);
        assert.deepStrictEqual(requests.map(request => [request.method, request.url]), [
            ["GET", "/daemon/v1/status"],
            ["POST", "/daemon/v1/projects/open"],
            ["GET", "/daemon/v1/status"],
            ["GET", "/daemon/v1/status"],
            ["POST", "/daemon/v1/shutdown"],
            ["GET", "/daemon/v1/status"],
            ["GET", "/daemon/v1/status"],
        ]);
    } finally {
        for (const socket of sockets) {
            socket.destroy();
        }
        await new Promise<void>(resolve => server.close(() => resolve()));
        fs.rmSync(root, { recursive: true, force: true });
    }
}

async function runDaemonAuthHeaderTest() {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "refact-daemon-auth-"));
    const sockets = new Set<Socket>();
    const requests: Array<{ method: string; url: string; authorization?: string }> = [];
    let spawned = false;
    const server = http.createServer((request, response) => {
        requests.push({
            method: request.method ?? "",
            url: request.url ?? "",
            authorization: request.headers.authorization,
        });
        response.setHeader("Connection", "close");
        response.setHeader("Content-Type", "application/json");
        if (request.url === "/daemon/v1/status") {
            const version = spawned ? "8.1.0" : "8.0.0";
            response.end(JSON.stringify(daemonStatus(version, (server.address() as AddressInfo).port)));
            return;
        }
        if (request.headers.authorization !== "Bearer secret-token") {
            response.statusCode = 401;
            response.end(JSON.stringify({ error: "missing auth" }));
            return;
        }
        if (request.url === "/daemon/v1/projects/open") {
            response.end(openProjectResponse("project-auth", root));
            return;
        }
        if (request.url === "/daemon/v1/shutdown") {
            response.end(JSON.stringify({ ok: true }));
            return;
        }
        response.statusCode = 404;
        response.end(JSON.stringify({ error: "not found" }));
    });
    server.on("connection", socket => {
        sockets.add(socket);
        socket.once("close", () => sockets.delete(socket));
    });
    server.keepAliveTimeout = 1;
    server.headersTimeout = 1000;
    await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
    try {
        const port = (server.address() as AddressInfo).port;
        const daemonJsonPath = path.join(root, "daemon.json");
        fs.writeFileSync(daemonJsonPath, daemonJson(port, "secret-token"));
        const refactPath = path.join(root, "refact");
        writeExecutable(refactPath, "");

        const status = await findExistingDaemon({ port, pluginVersion: "8.1.0", daemonJsonPath });
        assert.strictEqual(status, undefined);
        await openProject(root, { port, daemonJsonPath });
        const upgraded = await ensureDaemon(refactPath, {
            port,
            pluginVersion: "8.1.0",
            daemonJsonPath,
            timeoutMs: 1000,
            shutdownTimeoutMs: 1000,
            shutdownPollMs: 1,
            spawnDaemon: () => { spawned = true; },
            isProcessRunning: () => false,
        });

        assert.strictEqual(upgraded.version, "8.1.0");
        assert.strictEqual(spawned, true);
        assert.deepStrictEqual(requests.slice(0, 4).map(request => request.authorization), [
            undefined,
            "Bearer secret-token",
            undefined,
            "Bearer secret-token",
        ]);
        assert.deepStrictEqual(requests.slice(0, 4).map(request => [request.method, request.url]), [
            ["GET", "/daemon/v1/status"],
            ["POST", "/daemon/v1/projects/open"],
            ["GET", "/daemon/v1/status"],
            ["POST", "/daemon/v1/shutdown"],
        ]);
    } finally {
        for (const socket of sockets) {
            socket.destroy();
        }
        await new Promise<void>(resolve => server.close(() => resolve()));
        fs.rmSync(root, { recursive: true, force: true });
    }
}

async function runOpenProjectStaleTokenRetryTest() {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "refact-open-stale-token-"));
    const daemonJsonPath = path.join(root, "daemon.json");
    const sockets = new Set<Socket>();
    const requests: Array<{ method: string; url: string; authorization?: string }> = [];
    let openAttempts = 0;
    fs.writeFileSync(daemonJsonPath, daemonJson(1, "stale-token"));
    const server = http.createServer((request, response) => {
        requests.push({
            method: request.method ?? "",
            url: request.url ?? "",
            authorization: request.headers.authorization,
        });
        response.setHeader("Connection", "close");
        response.setHeader("Content-Type", "application/json");
        if (request.url === "/daemon/v1/status") {
            response.end(JSON.stringify(daemonStatus("8.1.0", (server.address() as AddressInfo).port)));
            return;
        }
        if (request.url === "/daemon/v1/projects/open") {
            openAttempts++;
            if (request.headers.authorization === "Bearer stale-token") {
                fs.writeFileSync(daemonJsonPath, daemonJson((server.address() as AddressInfo).port, "fresh-token"));
                response.statusCode = 401;
                response.end(JSON.stringify({ error: "stale token" }));
                return;
            }
            if (request.headers.authorization === "Bearer fresh-token") {
                response.end(openProjectResponse("project-fresh", root));
                return;
            }
        }
        response.statusCode = 401;
        response.end(JSON.stringify({ error: "missing auth" }));
    });
    server.on("connection", socket => {
        sockets.add(socket);
        socket.once("close", () => sockets.delete(socket));
    });
    await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
    try {
        const port = (server.address() as AddressInfo).port;
        fs.writeFileSync(daemonJsonPath, daemonJson(port, "stale-token"));
        const status = await findExistingDaemon({ port, daemonJsonPath, pluginVersion: "8.1.0" });
        assert.strictEqual(status?.authToken, "stale-token");

        const opened = await openProject(root, { port, authToken: status?.authToken, daemonJsonPath, timeoutMs: 1000 });

        assert.strictEqual(opened["project_id"], "project-fresh");
        assert.strictEqual(openAttempts, 2);
        assert.deepStrictEqual(requests.map(request => request.authorization), [
            undefined,
            "Bearer stale-token",
            "Bearer fresh-token",
        ]);
    } finally {
        for (const socket of sockets) {
            socket.destroy();
        }
        await new Promise<void>(resolve => server.close(() => resolve()));
        fs.rmSync(root, { recursive: true, force: true });
    }
}

async function runShutdownTokenRotationTest() {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "refact-shutdown-token-"));
    const daemonJsonPath = path.join(root, "daemon.json");
    const refactPath = path.join(root, "refact");
    const sockets = new Set<Socket>();
    const requests: Array<{ method: string; url: string; authorization?: string }> = [];
    let spawned = false;
    fs.writeFileSync(daemonJsonPath, daemonJson(1, "stale-token"));
    writeExecutable(refactPath, "");
    const server = http.createServer((request, response) => {
        requests.push({
            method: request.method ?? "",
            url: request.url ?? "",
            authorization: request.headers.authorization,
        });
        response.setHeader("Connection", "close");
        response.setHeader("Content-Type", "application/json");
        if (request.url === "/daemon/v1/status") {
            response.end(JSON.stringify(daemonStatus(spawned ? "8.1.0" : "8.0.0", (server.address() as AddressInfo).port)));
            return;
        }
        if (request.url === "/daemon/v1/shutdown") {
            if (request.headers.authorization === "Bearer stale-token") {
                fs.writeFileSync(daemonJsonPath, daemonJson((server.address() as AddressInfo).port, "fresh-token"));
                response.statusCode = 401;
                response.end(JSON.stringify({ error: "stale token" }));
                return;
            }
            if (request.headers.authorization === "Bearer fresh-token") {
                response.end(JSON.stringify({ ok: true }));
                return;
            }
        }
        response.statusCode = 401;
        response.end(JSON.stringify({ error: "missing auth" }));
    });
    server.on("connection", socket => {
        sockets.add(socket);
        socket.once("close", () => sockets.delete(socket));
    });
    await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
    try {
        const port = (server.address() as AddressInfo).port;
        fs.writeFileSync(daemonJsonPath, daemonJson(port, "stale-token"));
        const upgraded = await ensureDaemon(refactPath, {
            port,
            daemonJsonPath,
            pluginVersion: "8.1.0",
            timeoutMs: 1000,
            shutdownTimeoutMs: 1000,
            shutdownPollMs: 1,
            spawnDaemon: () => { spawned = true; },
            isProcessRunning: () => false,
        });

        assert.strictEqual(upgraded.version, "8.1.0");
        assert.deepStrictEqual(
            requests
                .filter(request => request.url === "/daemon/v1/shutdown")
                .map(request => request.authorization),
            ["Bearer stale-token", "Bearer fresh-token"],
        );
    } finally {
        for (const socket of sockets) {
            socket.destroy();
        }
        await new Promise<void>(resolve => server.close(() => resolve()));
        fs.rmSync(root, { recursive: true, force: true });
    }
}

async function runDiskPortTokenMismatchRecoveryTest() {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "refact-port-token-mismatch-"));
    const daemonJsonPath = path.join(root, "daemon.json");
    const sockets = new Set<Socket>();
    const requests: Array<{ method: string; url: string; authorization?: string }> = [];
    const server = http.createServer((request, response) => {
        requests.push({
            method: request.method ?? "",
            url: request.url ?? "",
            authorization: request.headers.authorization,
        });
        response.setHeader("Connection", "close");
        response.setHeader("Content-Type", "application/json");
        if (request.url === "/daemon/v1/status") {
            response.end(JSON.stringify(daemonStatus("8.1.0", (server.address() as AddressInfo).port)));
            return;
        }
        if (request.url === "/daemon/v1/projects/open") {
            if (request.headers.authorization === undefined) {
                fs.writeFileSync(daemonJsonPath, daemonJson((server.address() as AddressInfo).port, "active-token"));
                response.statusCode = 401;
                response.end(JSON.stringify({ error: "missing auth" }));
                return;
            }
            if (request.headers.authorization === "Bearer active-token") {
                response.end(openProjectResponse("project-active", root));
                return;
            }
        }
        response.statusCode = 401;
        response.end(JSON.stringify({ error: "wrong token" }));
    });
    server.on("connection", socket => {
        sockets.add(socket);
        socket.once("close", () => sockets.delete(socket));
    });
    await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
    try {
        const port = (server.address() as AddressInfo).port;
        fs.writeFileSync(daemonJsonPath, daemonJson(port + 1, "wrong-port-token"));
        const status = await findExistingDaemon({ port, daemonJsonPath, pluginVersion: "8.1.0" });
        assert.strictEqual(status?.port, port);
        assert.strictEqual(status?.authToken, null);

        const opened = await openProject(root, { port, authToken: status?.authToken, daemonJsonPath, timeoutMs: 1000 });

        assert.strictEqual(opened["project_id"], "project-active");
        assert.deepStrictEqual(requests.map(request => request.authorization), [undefined, undefined, "Bearer active-token"]);
    } finally {
        for (const socket of sockets) {
            socket.destroy();
        }
        await new Promise<void>(resolve => server.close(() => resolve()));
        fs.rmSync(root, { recursive: true, force: true });
    }
}

async function runSpawnFailureSurfacesLogTest() {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "refact-spawn-error-"));
    try {
        const daemonJsonPath = path.join(root, "daemon.json");
        const refactPath = path.join(root, "refact");
        writeExecutable(refactPath, "#!/definitely/missing/refact-interpreter\n");
        const logPath = path.join(root, "logs", "daemon.log");
        fs.mkdirSync(path.dirname(logPath), { recursive: true });
        fs.writeFileSync(logPath, "startup explosion from daemon.log\n");
        let ensureError: Error | undefined;

        try {
            await ensureDaemon(refactPath, {
                daemonJsonPath,
                timeoutMs: 1000,
                readDaemonInfo: async () => undefined,
            });
        } catch (error) {
            ensureError = error instanceof Error ? error : new Error(String(error));
        }

        assert.strictEqual(ensureError instanceof Error, true);
        assert.strictEqual(ensureError?.message.includes("Failed to start Refact daemon"), true);
        assert.strictEqual(ensureError?.message.includes(refactPath), true);
        assert.strictEqual(ensureError?.message.includes(logPath), true);
        assert.strictEqual(ensureError?.message.includes("startup explosion from daemon.log"), true);
    } finally {
        fs.rmSync(root, { recursive: true, force: true });
    }
}

function writeExecutable(filePath: string, contents: string): void {
    fs.writeFileSync(filePath, contents, { mode: 0o755 });
    if (process.platform !== "win32") {
        fs.chmodSync(filePath, 0o755);
    }
}

function daemonJson(port: number, authToken: string): string {
    return JSON.stringify({ port, ["auth_token"]: authToken });
}

function openProjectResponse(projectId: string, root: string): string {
    return JSON.stringify({ ["project_id"]: projectId, slug: projectId, root, pinned: false });
}

function daemonStatus(version = "8.1.0", port = 8488, authToken?: string): DaemonStatus {
    const status = {} as DaemonStatus;
    status.pid = 1;
    status.version = version;
    status.port = port;
    status.started_at_ms = 0;
    status.uptime_secs = 0;
    status.workers = 0;
    status.authToken = authToken;
    return status;
}

runRefactDaemonTests().catch(error => {
    console.error(error);
    process.exitCode = 1;
});

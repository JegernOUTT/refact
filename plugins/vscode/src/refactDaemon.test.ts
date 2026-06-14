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
    assert.strictEqual(compareVersions("8.1.0", "8.1.0-alpha.1"), 0);
    assert.strictEqual(compareVersions("8.1.0", "8.1.1"), -1);
    assert.strictEqual(isPluginNewerThanDaemon("8.2.0", "8.1.9"), true);
    assert.strictEqual(isPluginNewerThanDaemon("8.1.0", "8.1.0"), false);

    await runBundledRefactSpawnTests();
    await runStandaloneResolutionTests();
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

        const pathDir = path.join(root, "path-bin");
        const homeDir = path.join(root, "home");
        const cacheDir = path.join(root, "cache");
        const pathRefact = path.join(pathDir, process.platform === "win32" ? "refact.exe" : "refact");
        const homeRefact = path.join(homeDir, ".refact", "bin", process.platform === "win32" ? "refact.exe" : "refact");
        fs.mkdirSync(path.dirname(pathRefact), { recursive: true });
        fs.mkdirSync(path.dirname(homeRefact), { recursive: true });
        fs.writeFileSync(pathRefact, "");
        fs.writeFileSync(homeRefact, "");

        const oldPathIsSkipped = await resolveRefactBinary({
            minVersion: "8.1.0",
            pinnedVersion: "8.1.0",
            cacheDir,
            pathEnv: pathDir,
            homeDir,
            runVersion: async binPath => binPath === pathRefact ? "refact 8.0.0" : "refact 8.1.0",
        });
        assert.strictEqual(oldPathIsSkipped, homeRefact);

        const downloads: string[] = [];
        const extracted = await resolveRefactBinary({
            minVersion: "8.1.0",
            pinnedVersion: "8.1.0",
            cacheDir,
            pathEnv: pathDir,
            homeDir,
            platform: "linux",
            arch: "x64",
            runVersion: async binPath => binPath.includes(`${path.sep}cache${path.sep}`) ? "refact 8.1.0" : "refact 7.9.0",
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
                fs.writeFileSync(path.join(destDir, "refact"), "");
            },
            chmod: async () => undefined,
        });
        assert.strictEqual(extracted, path.join(cacheDir, "8.1.0", "x86_64-unknown-linux-gnu", "refact"));
        assert.deepStrictEqual(downloads, [
            "https://github.com/JegernOUTT/refact/releases/download/engine/v8.1.0/refact-8.1.0-x86_64-unknown-linux-gnu.tar.gz",
            "https://github.com/JegernOUTT/refact/releases/download/engine/v8.1.0/refact-8.1.0-x86_64-unknown-linux-gnu.tar.gz.sha256",
        ]);
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
            isProcessRunning: () => oldStatusReadsAfterShutdown < 3,
            sleep: async ms => {
                sleeps.push(ms);
                time += ms;
            },
            now: () => time,
        });

        assert.strictEqual(status.version, "8.2.0");
        assert.strictEqual(spawned, true);
        assert.deepStrictEqual(sleeps, [200, 200]);
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

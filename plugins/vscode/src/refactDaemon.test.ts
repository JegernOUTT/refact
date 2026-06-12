import * as assert from "assert";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import {
    browserProjectUrl,
    compareVersions,
    daemonOpenProjectUrl,
    daemonStatusUrl,
    ensureBundledRefactPath,
    ensureDaemon,
    isPluginNewerThanDaemon,
    missingBundledRefactError,
    projectProxyBaseUrl,
    resolveBundledRefactPath,
    type DaemonStatus,
} from "./refactDaemon";

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
}

async function runBundledRefactSpawnTests() {
    const assetPath = fs.mkdtempSync(path.join(os.tmpdir(), "refact-daemon-test-"));
    try {
        const refactPath = resolveBundledRefactPath(assetPath);
        const legacyPath = path.join(assetPath, process.platform === "win32" ? "refact-lsp.exe" : "refact-lsp");
        fs.writeFileSync(refactPath, "");
        fs.writeFileSync(legacyPath, "");

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
        assert.notStrictEqual(spawned[0], legacyPath);

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

        assert.strictEqual(ensureError?.message, missingBundledRefactError(assetPath));
        assert.strictEqual(readAttempts, 0);
        assert.deepStrictEqual(spawned, [path.join(assetPath, process.platform === "win32" ? "refact.exe" : "refact")]);

        ensureError = undefined;
        try {
            await ensureDaemon(legacyPath, {
                timeoutMs: 1,
                spawnDaemon: binPath => spawned.push(binPath),
                readDaemonInfo: async () => undefined,
                sleep: async () => undefined,
            });
        } catch (error) {
            ensureError = error instanceof Error ? error : new Error(String(error));
        }

        assert.strictEqual(ensureError?.message, missingBundledRefactError(assetPath));
        assert.deepStrictEqual(spawned, [path.join(assetPath, process.platform === "win32" ? "refact.exe" : "refact")]);
    } finally {
        fs.rmSync(assetPath, { recursive: true, force: true });
    }
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

function daemonStatus(): DaemonStatus {
    const status = {} as DaemonStatus;
    status.pid = 1;
    status.version = "8.1.0";
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

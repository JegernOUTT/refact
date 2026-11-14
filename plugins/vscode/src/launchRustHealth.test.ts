import * as assert from "assert";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as resolver from "./refactBinaryResolver";
import { resolveBundledRefactPath } from "./refactDaemon";
import {
    WorkerHealthProbe,
    type WorkerHealthScheduler,
} from "./launchRustHealth";

type FakeTimer = number;

type ScheduledTimer = {
    id: FakeTimer;
    callback: () => void;
    delayMs: number;
};

class FakeScheduler implements WorkerHealthScheduler<FakeTimer> {
    private nextId = 1;
    public scheduled: ScheduledTimer[] = [];
    public clears: FakeTimer[] = [];

    setTimeout(callback: () => void, delayMs: number): FakeTimer {
        const id = this.nextId++;
        this.scheduled.push({ id, callback, delayMs });
        return id;
    }

    clearTimeout(timer: FakeTimer): void {
        this.clears.push(timer);
        this.scheduled = this.scheduled.filter(entry => entry.id !== timer);
    }

    pendingDelay(): number | undefined {
        return this.scheduled[0]?.delayMs;
    }

    pendingCount(): number {
        return this.scheduled.length;
    }

    async fireNext(): Promise<number | undefined> {
        const next = this.scheduled.shift();
        if (!next) {
            return undefined;
        }
        next.callback();
        await flushMicrotasks();
        return next.delayMs;
    }
}

function flushMicrotasks(): Promise<void> {
    return new Promise(resolve => setImmediate(resolve));
}

function controllableProbe(results: boolean[], fallback = true): { probe: () => Promise<boolean>; calls: () => number } {
    const queue = [...results];
    let calls = 0;
    return {
        probe: async () => {
            calls++;
            return queue.length > 0 ? queue.shift() ?? fallback : fallback;
        },
        calls: () => calls,
    };
}

async function transientFailureDoesNotRecover() {
    const scheduler = new FakeScheduler();
    let recoverCalls = 0;
    const { probe, calls } = controllableProbe([false, true, true]);
    const health = new WorkerHealthProbe<FakeTimer>({
        probe,
        recover: () => { recoverCalls++; },
        scheduler,
        probeIntervalMs: 1000,
        failureThreshold: 3,
        initialBackoffMs: 1000,
        maxBackoffMs: 30000,
    });

    health.start();
    assert.strictEqual(scheduler.pendingDelay(), 1000);

    await scheduler.fireNext();
    assert.strictEqual(recoverCalls, 0);
    assert.strictEqual(scheduler.pendingDelay(), 1000);

    await scheduler.fireNext();
    assert.strictEqual(recoverCalls, 0);
    assert.strictEqual(scheduler.pendingDelay(), 1000);

    assert.strictEqual(calls(), 2);
    health.dispose();
}

async function sustainedFailureRecoversWithBackoff() {
    const scheduler = new FakeScheduler();
    let recoverCalls = 0;
    const { probe } = controllableProbe([], false);
    const health = new WorkerHealthProbe<FakeTimer>({
        probe,
        recover: () => { recoverCalls++; },
        scheduler,
        probeIntervalMs: 1000,
        failureThreshold: 3,
        initialBackoffMs: 1000,
        maxBackoffMs: 4000,
    });

    health.start();

    const backoffDelays: number[] = [];
    for (let recovery = 0; recovery < 4; recovery++) {
        await scheduler.fireNext();
        await scheduler.fireNext();
        const recoverCallsBefore = recoverCalls;
        await scheduler.fireNext();
        assert.strictEqual(recoverCalls, recoverCallsBefore + 1);
        backoffDelays.push(scheduler.pendingDelay() ?? -1);
    }

    assert.deepStrictEqual(backoffDelays, [1000, 2000, 4000, 4000]);
    assert.strictEqual(recoverCalls, 4);
    health.dispose();
}

async function successResetsConsecutiveFailures() {
    const scheduler = new FakeScheduler();
    let recoverCalls = 0;
    const { probe } = controllableProbe([false, false, true, false, false, true]);
    const health = new WorkerHealthProbe<FakeTimer>({
        probe,
        recover: () => { recoverCalls++; },
        scheduler,
        probeIntervalMs: 1000,
        failureThreshold: 3,
        initialBackoffMs: 1000,
        maxBackoffMs: 30000,
    });

    health.start();
    for (let i = 0; i < 6; i++) {
        await scheduler.fireNext();
    }

    assert.strictEqual(recoverCalls, 0);
    assert.strictEqual(scheduler.pendingDelay(), 1000);
    health.dispose();
}

async function disposeStopsProbeWithoutLeakingTimers() {
    const scheduler = new FakeScheduler();
    let recoverCalls = 0;
    const { probe } = controllableProbe([], false);
    const health = new WorkerHealthProbe<FakeTimer>({
        probe,
        recover: () => { recoverCalls++; },
        scheduler,
        probeIntervalMs: 1000,
        failureThreshold: 3,
    });

    health.start();
    assert.strictEqual(scheduler.pendingCount(), 1);
    const scheduledId = scheduler.scheduled[0]?.id;

    health.dispose();
    assert.strictEqual(scheduler.pendingCount(), 0);
    assert.strictEqual(scheduler.clears.includes(scheduledId ?? -1), true);

    health.start();
    assert.strictEqual(scheduler.pendingCount(), 0);
    assert.strictEqual(recoverCalls, 0);
}

async function stopDuringInFlightProbeDoesNotRecover() {
    const scheduler = new FakeScheduler();
    let recoverCalls = 0;
    let resolveProbe: ((value: boolean) => void) | undefined;
    const health = new WorkerHealthProbe<FakeTimer>({
        probe: () => new Promise<boolean>(resolve => { resolveProbe = resolve; }),
        recover: () => { recoverCalls++; },
        scheduler,
        probeIntervalMs: 1000,
        failureThreshold: 1,
    });

    health.start();
    const pending = scheduler.scheduled.shift();
    assert.notStrictEqual(pending, undefined);
    pending?.callback();
    await flushMicrotasks();

    health.stop();
    resolveProbe?.(false);
    await flushMicrotasks();

    assert.strictEqual(recoverCalls, 0);
    assert.strictEqual(scheduler.pendingCount(), 0);
    health.dispose();
}

/* eslint-disable @typescript-eslint/naming-convention */
async function runtimeMetadataSelectsBundledPrerelease() {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "refact-runtime-metadata-"));
    const moduleLoader = require("module") as { _load: (request: string, parent: { filename?: string } | undefined, isMain: boolean) => unknown };
    const originalLoad = moduleLoader._load;
    const modulePath = require.resolve("./launchRust");
    const previousModule = require.cache[modulePath];
    const originalBar = global.status_bar;
    const originalPanel = global.side_panel;
    const originalTabs = global.open_chat_tabs;
    const originalCaps = global.have_caps;
    const runtimeVersion = "8.7.0-main-4053-d8e6abb9";
    let daemonVersion = runtimeVersion;
    let metadata: Record<string, unknown> = { version: "8.7.0", refactRuntimeVersion: runtimeVersion };
    let existing = true;
    let localMissing = false;
    let uncheckedVersion = false;
    const requirements: string[] = [];
    const versionChecks: string[] = [];
    let resolutionCalls = 0;
    const bundledDir = path.join(root, "assets");
    const bundledPath = resolveBundledRefactPath(bundledDir);
    const homeDir = path.join(root, "home");
    const sharedPath = resolver.sharedRefactBinaryPath(homeDir, "linux");
    const pathDir = path.join(root, "path");
    const pathBinary = path.join(pathDir, "refact");
    let unexpectedNetworkCalls = 0;
    const unexpected = async (): Promise<never> => {
        unexpectedNetworkCalls++;
        throw new Error("downloads/releases must not be called");
    };
    const resolveOptions = (options: resolver.RefactBinaryResolverOptions): resolver.RefactBinaryResolverOptions => ({
        ...options, homeDir, pathEnv: pathDir, platform: "linux", arch: "x64",
        runVersion: async binPath => { versionChecks.push(binPath); return fs.readFileSync(binPath, "utf8"); },
        downloadFile: unexpected, listPublishedEngineVersions: unexpected,
        onDownloadStart: () => { throw new Error("download must not start"); },
    });
    const daemon = {
        ...require("./refactDaemon") as typeof import("./refactDaemon"),
        daemonEndpoints: () => [],
        sha256OfFile: async () => "test-hash",
        findExistingDaemon: async (options: { pluginVersion: string }) => {
            requirements.push(options.pluginVersion);
            return existing ? { port: 9234, version: daemonVersion } : undefined;
        },
        ensureDaemon: async (_binPath: string, options: { pluginVersion: string }) => {
            requirements.push(options.pluginVersion);
            return { port: 9234, version: daemonVersion };
        },
        openProject: async () => ({ project_id: "test", worker: { lsp_port: 9001 } }),
    };
    const mocks: Record<string, unknown> = {
        vscode: {
            workspace: {
                workspaceFolders: [{ uri: { scheme: "file", fsPath: root } }],
                getConfiguration: () => ({ get: () => undefined, inspect: () => undefined }),
            },
            window: { createOutputChannel: () => ({}) },
            extensions: { getExtension: () => ({ packageJSON: metadata }) },
        },
        "vscode-languageclient/node": { RevealOutputChannelOn: { Error: 1 } },
        "./fetchAPI": {}, "./rconsoleCommands": {}, "./quickProvider": {},
        "./refactPathRegistration": {}, "./refactDaemon": daemon,
        "./refactBinaryResolver": {
            ...resolver,
            resolveLocalRefactBinaryDetailedOrNull: async (options: resolver.RefactBinaryResolverOptions) => {
                resolutionCalls++;
                assert.strictEqual(options.minVersion, options.pinnedVersion);
                if (localMissing) { return undefined; }
                const result = await resolver.resolveLocalRefactBinaryDetailedOrNull(resolveOptions(options));
                return uncheckedVersion && result ? { binPath: result.binPath } : result;
            },
            resolveRefactBinaryDetailed: async (options: resolver.RefactBinaryResolverOptions) => {
                assert.strictEqual(options.minVersion, options.pinnedVersion);
                const result = await resolver.resolveRefactBinaryDetailed(resolveOptions(options));
                return uncheckedVersion ? { binPath: result.binPath } : result;
            },
            resolveCompatibleSharedRefactBinaryOrNull: async (_home: string, minVersion: string) => {
                requirements.push(minVersion);
                return resolver.resolveCompatibleSharedRefactBinaryOrNull(homeDir, minVersion, "linux",
                    async binPath => fs.readFileSync(binPath, "utf8"));
            },
        },
    };
    try {
        for (const candidate of [bundledPath, sharedPath, pathBinary]) {
            fs.mkdirSync(path.dirname(candidate), { recursive: true });
            fs.writeFileSync(candidate, `refact ${candidate === bundledPath ? runtimeVersion : "8.7.0"}`);
        }
        moduleLoader._load = function(request, parent, isMain) {
            return parent?.filename === modulePath && request in mocks ? mocks[request] : originalLoad.call(this, request, parent, isMain);
        };
        global.status_bar = { choose_color() {}, set_socket_error() {} } as unknown as typeof global.status_bar;
        global.side_panel = undefined;
        global.open_chat_tabs = [];
        delete require.cache[modulePath];
        const { RustBinaryBlob } = require("./launchRust") as typeof import("./launchRust");
        const blob = new RustBinaryBlob(bundledDir, path.join(root, "cache"));
        blob.stop_lsp = async () => undefined;
        blob.start_lsp_socket = async () => undefined;
        await blob.launch();
        assert.strictEqual(blob.connection_issue(), null);
        assert.deepStrictEqual(versionChecks, [bundledPath], "matching prerelease must win before shared/PATH stable versions");
        assert.strictEqual(blob.runtime_versions().plugin?.version, "8.7.0");
        assert.strictEqual(blob.runtime_versions().requestedVersion, runtimeVersion);
        assert.strictEqual(blob.runtime_versions().selected?.version, runtimeVersion);
        assert.deepStrictEqual(requirements, [runtimeVersion, runtimeVersion]);

        existing = false;
        localMissing = true;
        uncheckedVersion = true;
        requirements.length = 0;
        versionChecks.length = 0;
        await blob.settings_changed();
        assert.strictEqual(blob.connection_issue(), null);
        assert.deepStrictEqual(versionChecks, [bundledPath]);
        assert.deepStrictEqual(requirements, [runtimeVersion, runtimeVersion], "find and ensure must fall back to runtime requirement, not IDE version");

        const unrelatedRuntimeVersion = "9.2.1-rc.2";
        metadata = { version: "8.7.0", refactRuntimeVersion: unrelatedRuntimeVersion };
        daemonVersion = unrelatedRuntimeVersion;
        existing = true;
        localMissing = false;
        uncheckedVersion = false;
        fs.writeFileSync(bundledPath, `refact ${unrelatedRuntimeVersion}`);
        requirements.length = 0;
        versionChecks.length = 0;
        await blob.settings_changed();
        assert.strictEqual(blob.connection_issue(), null);
        assert.strictEqual(blob.runtime_versions().plugin?.version, "8.7.0");
        assert.strictEqual(blob.runtime_versions().requestedVersion, unrelatedRuntimeVersion);
        assert.strictEqual(blob.runtime_versions().selected?.version, unrelatedRuntimeVersion);
        assert.strictEqual(blob.runtime_versions().daemon?.version, unrelatedRuntimeVersion);
        assert.deepStrictEqual(versionChecks, [bundledPath]);
        assert.deepStrictEqual(requirements, [unrelatedRuntimeVersion, unrelatedRuntimeVersion]);

        for (const invalid of [undefined, null, "", "8.7", "v8.7.0", " 8.7.0", "8.7.0\n", "8.7.0-main_abc", "8.7.0-01", "08.7.0", "8.7.0/../../x", 870, {}, []]) {
            metadata = { version: "8.7.0", refactRuntimeVersion: invalid };
            const before = resolutionCalls;
            requirements.length = 0;
            await blob.settings_changed();
            assert.deepStrictEqual(blob.connection_issue(), { code: "engine_start_failed", stage: "engine_resolution" });
            assert.strictEqual(resolutionCalls, before, "invalid present metadata must fail before binary discovery");
            assert.deepStrictEqual(requirements, []);
            assert.strictEqual(blob.get_port(), 0);
        }

        metadata = { version: "8.7.0" };
        daemonVersion = "8.7.0";
        fs.writeFileSync(bundledPath, `refact ${runtimeVersion}`);
        existing = true;
        localMissing = false;
        uncheckedVersion = false;
        requirements.length = 0;
        versionChecks.length = 0;
        await blob.settings_changed();
        assert.strictEqual(blob.connection_issue(), null);
        assert.strictEqual(blob.runtime_versions().plugin?.version, "8.7.0");
        assert.strictEqual(blob.runtime_versions().requestedVersion, "8.7.0");
        assert.strictEqual(blob.runtime_versions().selected?.version, "8.7.0");
        assert.strictEqual(blob.runtime_versions().daemon?.version, "8.7.0");
        assert.deepStrictEqual(requirements, ["8.7.0", "8.7.0"]);
        assert.deepStrictEqual(versionChecks, [bundledPath, sharedPath], "absent metadata retains legacy strict prerelease rejection");
        assert.strictEqual(unexpectedNetworkCalls, 0);
        console.log("Runtime metadata: bundled prerelease, daemon requirement, invalid-present failure and absent legacy behavior passed");
    } finally {
        moduleLoader._load = originalLoad;
        global.status_bar = originalBar;
        global.side_panel = originalPanel;
        global.open_chat_tabs = originalTabs;
        global.have_caps = originalCaps;
        if (previousModule) { require.cache[modulePath] = previousModule; } else { delete require.cache[modulePath]; }
        fs.rmSync(root, { recursive: true, force: true });
    }
}

/* eslint-enable @typescript-eslint/naming-convention */

export async function runLaunchRustHealthTests() {
    await runtimeMetadataSelectsBundledPrerelease();
    await transientFailureDoesNotRecover();
    await sustainedFailureRecoversWithBackoff();
    await successResetsConsecutiveFailures();
    await disposeStopsProbeWithoutLeakingTimers();
    await stopDuringInFlightProbeDoesNotRecover();
}

runLaunchRustHealthTests().catch(error => {
    console.error(error);
    process.exitCode = 1;
});

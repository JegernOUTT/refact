/* eslint-disable @typescript-eslint/no-explicit-any, @typescript-eslint/naming-convention */
import * as assert from "assert";
import { EventEmitter } from "events";
import { WorkspaceBusyError } from "./refactDaemon";
import { backendConfigForStatus } from "./backendStatus";

// Load the actual lifecycle class with inert IDE/network dependencies. No engine,
// socket, filesystem discovery, timer or process operation escapes these mocks.
export async function runRustBinaryBlobLifecycleTests() {
    const moduleLoader = require("module") as any;
    const originalLoad = moduleLoader._load;
    const originalTimeout = global.setTimeout;
    const originalClear = global.clearTimeout;
    const originalPanel = global.side_panel;
    const originalBar = global.status_bar;
    const originalTabs = global.open_chat_tabs;
    const originalCaps = global.have_caps;
    const timers = new Map<object, { callback: () => void; ms: number }>();
    const calls = { resolve: 0, find: 0, download: 0, ensure: 0, open: 0 };
    let folders: any[] = [];
    let configuredPort = 9234;
    let mode = "ready";
    let releaseReady: (() => void) | undefined;
    let releaseCaps: (() => void) | undefined;
    let releaseOpen: (() => void) | undefined;
    const snapshots: any[] = [];
    let blob: any;
    let localVersion: string | undefined = "8.8.0";
    let releaseBuild: ((value: any) => void) | undefined;
    const socketErrors: string[] = [];
    class FakeSocket extends EventEmitter {
        destroyed = false;
        connect() {
            if (mode === "connection_refused") {
                this.emit("error", new Error("connect ECONNREFUSED"));
                this.emit("close");
                return;
            }
            this.emit("connect");
        }
        destroy() { this.destroyed = true; }
    }
    class FakeClient {
        start() { return { dispose() {} }; }
        async onReady() {
            if (mode === "lsp_failure") { throw new Error("LSP initialization failed"); }
            if (mode === "ready_deferred") { await new Promise<void>(resolve => { releaseReady = resolve; }); }
        }
        async stop() {}
    }
    const daemon = {
        ...require("./refactDaemon"),
        daemonEndpoints: () => [],
        sha256OfFile: async () => "inert-hash",
        findExistingDaemon: async () => {
            calls.find++;
            return mode === "missing" || mode === "ensure_failure" ? undefined : { port: configuredPort, version: "8.9.0" };
        },
        ensureDaemon: async () => { calls.ensure++; throw new Error("ensure failed"); },
        openProject: async () => {
            calls.open++;
            if (mode === "busy") {
                throw new WorkspaceBusyError({ code: "workspace_busy", holderPid: 4082485, holderHttpPort: 8005 });
            }
            if (mode === "generic") { throw new Error("SECRET_SENTINEL /private/path?token=secret"); }
            if (mode === "deferred") {
                await new Promise<void>(resolve => { releaseOpen = resolve; });
                throw new Error("obsolete failure");
            }
            return { project_id: "test", worker: { lsp_port: 9001 } };
        },
    };
    const mocks: Record<string, any> = {
        vscode: {
            workspace: {
                get workspaceFolders() { return folders; },
                getConfiguration: () => ({ get: (key: string) => key === "refactai.daemonPort" ? configuredPort : undefined, inspect: () => undefined }),
            },
            window: { createOutputChannel: () => ({}), showWarningMessage: () => undefined },
            extensions: { getExtension: () => ({ packageJSON: { version: "8.7.0" } }) },
        },
        "vscode-languageclient/node": { LanguageClient: FakeClient, RevealOutputChannelOn: { Error: 1 } },
        net: { Socket: FakeSocket },
        "fetch-h2": {
            Request: class {},
            fetch: async () => ({ status: 200, json: async () => {
                if (mode === "build_deferred") { return new Promise(resolve => { releaseBuild = resolve; }); }
                if (mode === "build_failure") { throw new Error("invalid build info"); }
                return { version: "9.0.0", commit: "worker-commit" };
            } }),
        },
        "./fetchAPI": {},
        "./rconsoleCommands": {},
        "./quickProvider": {},
        "./refactPathRegistration": {},
        "./refactDaemon": daemon,
        "./refactBinaryResolver": {
            resolveLocalRefactBinaryDetailedOrNull: async () => {
                calls.resolve++;
                if (mode === "resolution_failure") { throw new Error("resolution failed"); }
                return { binPath: "/inert/refact", version: localVersion };
            },
            resolveCompatibleSharedRefactBinaryOrNull: async () => null,
            resolveRefactBinaryDetailed: async () => { calls.download++; return { binPath: "/inert/refact" }; },
            isSharedRefactBinaryPath: () => false,
        },
    };
    const modulePath = require.resolve("./launchRust");
    const previousModule = require.cache[modulePath];
    try {
        moduleLoader._load = function(request: string, parent: any, isMain: boolean) {
            if (parent?.filename === modulePath && request in mocks) { return mocks[request]; }
            return originalLoad.call(this, request, parent, isMain);
        };
        global.setTimeout = ((callback: () => void, ms: number) => {
            const id = {};
            timers.set(id, { callback, ms });
            return id;
        }) as any;
        global.clearTimeout = ((id: object) => { timers.delete(id); }) as any;
        global.status_bar = { choose_color() {}, set_socket_error(_failed: boolean, description: string) { socketErrors.push(description); } } as any;
        global.open_chat_tabs = [];
        global.side_panel = { handleSettingsChange: () => {
            if (blob) { snapshots.push({ ...backendConfigForStatus(blob.backend_status(), blob.connection_issue()), daemonPort: blob.daemon_control_port(), runtimeVersions: blob.runtime_versions() }); }
        } } as any;
        delete require.cache[modulePath];
        const { RustBinaryBlob } = require("./launchRust") as typeof import("./launchRust");
        blob = new RustBinaryBlob("/inert/assets");
        blob.read_caps = async () => undefined;
        blob.fetch_toolbox_config = async () => ({});
        // Disable only periodic health polling, not lifecycle or socket code.
        blob.workerHealthProbe = { stop() {}, start() {}, dispose() {} };
        const flush = async () => { for (let n = 0; n < 40; n++) { await Promise.resolve(); } };
        const fireRetry = async () => {
            const entry = [...timers.entries()].find(([id]) => id === blob.reconnectTimer);
            assert.ok(entry, "a bounded reconnect timer exists");
            timers.delete(entry[0]);
            entry[1].callback();
            await flush();
        };

        await blob.launch();
        assert.deepStrictEqual(blob.connection_issue(), { code: "workspace_required" });
        assert.deepStrictEqual(calls, { resolve: 0, find: 0, download: 0, ensure: 0, open: 0 });
        assert.strictEqual(timers.size, 0);
        folders = [{ uri: { scheme: "file", fsPath: "/inert/workspace" } }];
        blob.workspace_folders_changed();
        blob.workspace_folders_changed();
        await flush();
        assert.strictEqual(calls.open, 1, "multiple webviews request only one folder-transition attach");
        assert.strictEqual(blob.backend_status(), "ready");
        assert.strictEqual(blob.connection_issue(), null);
        await flush();
        const versions = blob.runtime_versions();
        assert.strictEqual(versions.requestedVersion, "8.7.0");
        assert.strictEqual(versions.plugin.version, "8.7.0");
        assert.strictEqual(versions.selected.version, "8.8.0");
        assert.strictEqual(versions.daemon.version, "8.9.0");
        assert.strictEqual(versions.worker.version, "9.0.0");
        assert.strictEqual(versions.worker.commit, "worker-commit");
        assert.strictEqual(versions.installed, null);
        assert.strictEqual(versions.staged, null);
        versions.worker.version = "mutated";
        assert.strictEqual(blob.runtime_versions().worker.version, "9.0.0");
        const readySocket = blob.lsp_socket;
        readySocket.emit("close");
        assert.strictEqual(blob.backend_status(), "connecting", "normal ready socket close remains transient");
        assert.strictEqual(blob.connection_issue(), null);
        assert.strictEqual(blob.runtime_versions().worker.state, "last_observed");
        mode = "missing";
        await fireRetry();
        assert.strictEqual(calls.ensure, 0, "automatic reconnect never upgrades/replaces an incompatible daemon");
        assert.strictEqual(calls.download, 0);
        assert.strictEqual(timers.size, 0);

        mode = "ensure_failure";
        await blob.settings_changed();
        assert.strictEqual(calls.ensure, 1);
        assert.strictEqual(timers.size, 0, "daemon startup failure requires an explicit action");
        mode = "generic";
        await blob.settings_changed();
        assert.strictEqual(blob.connection_issue().stage, "project_open");
        assert.ok(socketErrors.every(value => !value.includes("SECRET_SENTINEL") && !value.includes("token=secret")));
        assert.strictEqual(timers.size, 0, "generic project-open failure never retries automatically");
        mode = "resolution_failure";
        await blob.settings_changed();
        assert.strictEqual(blob.connection_issue().stage, "engine_resolution");
        assert.strictEqual(timers.size, 0, "resolution failure never redownloads automatically");
        mode = "lsp_failure";
        await blob.settings_changed();
        assert.strictEqual(blob.connection_issue().stage, "lsp_attach");
        assert.ok(timers.size > 0, "actual LSP attach failure retains automatic recovery");
        mode = "ready";
        await fireRetry();
        assert.strictEqual(blob.backend_status(), "ready");

        mode = "busy";
        await blob.settings_changed();
        assert.strictEqual(blob.get_port(), 0);
        assert.strictEqual(blob.rust_url(), "");
        assert.strictEqual(blob.browser_url(), "");
        assert.ok([...timers.values()].every(timer => timer.ms >= 5000));
        snapshots.length = 0;
        await fireRetry();
        assert.ok(snapshots.length > 0);
        assert.ok(snapshots.every(snapshot => snapshot.connectionIssue?.code === "workspace_busy" && !snapshot.backendReady), "busy issue survives all timer retry broadcasts");
        assert.strictEqual(calls.ensure, 1);
        mode = "ready";
        await fireRetry();
        assert.strictEqual(blob.backend_status(), "ready");
        assert.strictEqual(blob.connection_issue(), null);

        mode = "deferred";
        const obsolete = blob.settings_changed();
        await flush();
        assert.ok(releaseOpen);
        configuredPort = 9345;
        const current = blob.settings_changed();
        mode = "ready";
        snapshots.length = 0;
        releaseOpen();
        await obsolete;
        await current;
        assert.ok(snapshots.every(snapshot => snapshot.connectionStatus !== "failed"), "stale generation cannot publish its failure");
        assert.strictEqual(snapshots[0].daemonPort, 9345, "new control port is assigned before connecting broadcast");
        assert.strictEqual(blob.daemon_control_port(), 9345);
        localVersion = undefined;
        mode = "build_failure";
        await blob.settings_changed();
        await flush();
        assert.strictEqual(blob.backend_status(), "ready", "invalid build info does not block readiness");
        assert.strictEqual(blob.runtime_versions().selected.version, null, "unchecked explicit binary stays unknown");
        assert.strictEqual(blob.runtime_versions().worker, null);
        mode = "build_deferred";
        const endpointProbe = blob.probe_attached_worker();
        await flush();
        assert.ok(releaseBuild);
        blob.project_id = "replacement-project";
        releaseBuild!({ version: "WRONG_PROJECT" });
        await endpointProbe;
        assert.strictEqual(blob.runtime_versions().worker, null, "same-generation old project response is ignored");
        mode = "build_deferred";
        await blob.settings_changed();
        await flush();
        assert.ok(releaseBuild);
        mode = "build_failure";
        const replacement = blob.settings_changed();
        assert.strictEqual(blob.runtime_versions().worker, null, "explicit settings clears synchronously");
        assert.strictEqual(blob.runtime_versions().daemon, null);
        releaseBuild!({ version: "STALE", commit: "STALE" });
        await replacement;
        await flush();
        assert.strictEqual(blob.runtime_versions().worker, null, "superseded build response cannot cross generation");
        assert.ok(snapshots.every(snapshot => snapshot.runtimeVersions.worker?.version !== "STALE"));
        mode = "connection_refused";
        await blob.settings_changed();
        assert.deepStrictEqual(blob.connection_issue(), { code: "engine_start_failed", stage: "lsp_attach" }, "error plus close before connect is a startup failure");
        assert.strictEqual(blob.backend_status(), "failed");
        assert.ok(blob.reconnectTimer, "startup close retains bounded recovery");

        mode = "ready_deferred";
        const beforeReady = blob.settings_changed();
        await flush();
        const initializingSocket = blob.lsp_socket;
        initializingSocket.emit("close");
        await beforeReady;
        assert.deepStrictEqual(blob.connection_issue(), { code: "engine_start_failed", stage: "lsp_attach" }, "close during onReady is a startup failure");
        releaseReady!();
        await flush();
        assert.strictEqual(blob.backend_status(), "failed", "late onReady cannot revive closed socket");

        mode = "ready";
        blob.read_caps = () => new Promise<void>(resolve => { releaseCaps = resolve; });
        const duringCaps = blob.settings_changed();
        await flush();
        assert.strictEqual(blob.backend_status(), "ready");
        assert.ok(releaseCaps);
        blob.lsp_socket.emit("close");
        assert.strictEqual(blob.backend_status(), "connecting", "close after onReady during caps stays transient");
        assert.strictEqual(blob.connection_issue(), null);
        releaseCaps!();
        await duringCaps;
        assert.strictEqual(blob.connection_issue(), null);
        console.log("RustBinaryBlob mocked lifecycle: prerequisite, dedup, safe retries, busy persistence, transient close and stale generation passed");
    } finally {
        moduleLoader._load = originalLoad;
        global.setTimeout = originalTimeout;
        global.clearTimeout = originalClear;
        global.side_panel = originalPanel;
        global.status_bar = originalBar;
        global.open_chat_tabs = originalTabs;
        global.have_caps = originalCaps;
        if (previousModule) { require.cache[modulePath] = previousModule; } else { delete require.cache[modulePath]; }
    }
}

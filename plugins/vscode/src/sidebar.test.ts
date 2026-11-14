/* eslint-disable @typescript-eslint/naming-convention */
import * as assert from "assert";
import * as path from "path";
import {
    createCurrentProjectInfo,
    resolveFilePathWithinWorkspace,
} from "./sidebarPaths";
import { webviewEndpointConfig } from "./sidebarConfig";
import { backendConfigForStatus, effectiveLspPortForStatus } from "./backendStatus";
import { HostDecisionNotifications } from "./hostDecisions";
import { HOST_DECISION_PARITY_CASES, decisionQuestionsDigest, sanitizeEngineBaseUrl } from "refact-chat-js/dist/events";

export function runSidebarPathBoundaryTests() {
    const workspaceRoot = path.resolve("/workspace/repo");

    assert.strictEqual(
        resolveFilePathWithinWorkspace("src/index.ts", [workspaceRoot]),
        path.resolve(workspaceRoot, "src/index.ts"),
    );

    assert.strictEqual(
        resolveFilePathWithinWorkspace(path.resolve("/workspace/other/file.ts"), [workspaceRoot]),
        undefined,
    );

    assert.strictEqual(
        resolveFilePathWithinWorkspace("../other/file.ts", [workspaceRoot]),
        undefined,
    );

    assert.strictEqual(
        resolveFilePathWithinWorkspace(path.resolve("/workspace/repo2/file.ts"), [workspaceRoot]),
        undefined,
    );

    assert.strictEqual(
        resolveFilePathWithinWorkspace(path.resolve(workspaceRoot, "file.ts"), []),
        undefined,
    );

    assert.deepStrictEqual(createCurrentProjectInfo("repo", []), { name: "repo" });
}

function runWebviewEndpointConfigTests() {
    for (const code of ["workspace_required", "workspace_busy", "engine_start_failed"] as const) {
        const backend = backendConfigForStatus("failed", { code });
        const payload = { ...backend, lspPort: effectiveLspPortForStatus(8005, backend.connectionStatus), ...webviewEndpointConfig(backend.backendReady, "http://private/proxy", "http://private/?token=secret") };
        assert.deepStrictEqual(JSON.parse(JSON.stringify(payload)), { backendReady: false, connectionStatus: "failed", connectionIssue: { code }, lspPort: 0 });
        for (const status of ["connecting", "starting", "installing", "ready", "failed"] as const) {
            assert.strictEqual(backendConfigForStatus(status).connectionIssue, null);
        }
        assert.strictEqual(backendConfigForStatus("ready", { code }).connectionIssue, null);
    }
    const lspUrl = "http://127.0.0.1:8488/p/PID/";
    const browserUrl = "http://machine.local:8488/p/PID/";

    assert.deepStrictEqual(webviewEndpointConfig(true, lspUrl, browserUrl), { browserUrl, lspUrl });

    const startingConfig = webviewEndpointConfig(false, lspUrl, browserUrl);
    assert.deepStrictEqual(JSON.parse(JSON.stringify(startingConfig)), {});
    assert.strictEqual(startingConfig.lspUrl, undefined);
    assert.strictEqual(startingConfig.browserUrl, undefined);
    assert.strictEqual(Object.prototype.hasOwnProperty.call(startingConfig, "lspUrl"), true);
    assert.strictEqual(Object.prototype.hasOwnProperty.call(startingConfig, "browserUrl"), true);

    const missingProxyConfig = webviewEndpointConfig(true, "", browserUrl);
    assert.deepStrictEqual(missingProxyConfig, { browserUrl });
    assert.strictEqual(Object.prototype.hasOwnProperty.call(missingProxyConfig, "lspUrl"), false);

    const missingBrowserConfig = webviewEndpointConfig(true, lspUrl, "");
    assert.deepStrictEqual(missingBrowserConfig, { lspUrl });
    assert.strictEqual(Object.prototype.hasOwnProperty.call(missingBrowserConfig, "browserUrl"), false);
}

runSidebarPathBoundaryTests();
runWebviewEndpointConfigTests();

async function runHostDecisionNotificationTests() {
    assert.strictEqual(sanitizeEngineBaseUrl(" HTTP://LOCALHOST:80/p/one/?token=x#fragment "), "http://localhost/p/one");
    const scope = { projectRoot: "/workspace/repo", endpointIdentity: "http://localhost:8488/p/one" };
    let badge = 0;
    const notices: string[] = [];
    const opened: string[] = [];
    let select: (value: string | undefined) => void = () => {};
    const host = new HostDecisionNotifications({
        badge: count => { badge = count; },
        notify: message => {
            notices.push(message);
            return new Promise<string | undefined>(resolve => { select = resolve; });
        },
        openChat: chatId => { opened.push(chatId); },
    });
    for (const fixture of HOST_DECISION_PARITY_CASES) {
        assert.strictEqual(decisionQuestionsDigest(fixture.payload.questions), fixture.digest);
    }
    const ask = { ...HOST_DECISION_PARITY_CASES[0].payload, askedAtMs: 10, scope };
    const generation = host.bind(scope);
    host.ask(ask, generation);
    host.ask({ ...ask, origin: "native_chat_event" }, generation);
    assert.strictEqual(notices.length, 1, "stream and chat duplicate announce once");
    assert.strictEqual(badge, 1);
    select("Open Chat");
    await Promise.resolve();
    assert.deepStrictEqual(opened, [ask.chatId]);
    assert.strictEqual(badge, 1, "reading/opening is not native resolution");
    host.resolved({ ...ask, askedAtMs: 9, resolution: "answered" });
    assert.strictEqual(badge, 1, "old incarnation cannot resolve current ask");
    host.resolved({ ...ask, scope: { ...scope, projectRoot: "/other" }, resolution: "answered" });
    assert.strictEqual(badge, 1, "foreign root cannot resolve");
    host.resolved({ ...ask, resolution: "resolved_elsewhere" });
    assert.strictEqual(badge, 0);
    host.reconnect();
    host.ask(ask);
    assert.strictEqual(notices.length, 1, "reconnect replay cannot resurrect resolved question");
    host.bind(undefined);
    host.bind(scope);
    host.ask(ask);
    assert.strictEqual(notices.length, 1, "temporary endpoint outage retains tombstones");
    host.ask({ ...ask, askedAtMs: 11 });
    assert.strictEqual(notices.length, 2);
    host.bind({ ...scope, endpointIdentity: "http://localhost:8488/p/two" });
    select("Open Chat");
    await Promise.resolve();
    assert.deepStrictEqual(opened, [ask.chatId], "old endpoint notification action is inert");
    assert.strictEqual(badge, 0);
    host.ask(ask, generation);
    host.ask(ask);
    host.ask(null);
    host.ask({ chatId: "a", toolCallId: "b", questions: "bad" });
    host.ask({ ...ask, scope: undefined });
    host.resolved({ chatId: "a", toolCallId: "b", resolution: "approved" });
    assert.strictEqual(badge, 0);
    assert.strictEqual(notices.length, 2);
    const currentAsk = { ...ask, scope: { ...scope, endpointIdentity: "http://localhost:8488/p/two" } };
    host.ask(currentAsk);
    assert.strictEqual(badge, 1);
    host.taskDone({ chatId: "task-chat", summary: "Task completed" });
    select("Open Chat");
    await Promise.resolve();
    assert.strictEqual(badge, 1, "task notification and task Open Chat do not alter pending decisions");
    assert.strictEqual(opened[opened.length - 1], "task-chat");
    host.dispose();
    host.ask(ask);
    assert.strictEqual(badge, 0);
}

function runLegacyDecisionNotificationTests() {
    let badge = 0;
    let notices = 0;
    const host = new HostDecisionNotifications({
        badge: count => { badge = count; },
        notify: () => { notices++; return Promise.resolve(undefined); },
        openChat: () => { throw new Error("No navigation was selected"); },
    });
    host.bind({ projectRoot: "", endpointIdentity: "http://localhost:8488" });
    const ask = HOST_DECISION_PARITY_CASES[0].payload;
    host.resolved({ chatId: ask.chatId, toolCallId: ask.toolCallId, resolution: "answered" });
    host.ask(ask);
    assert.strictEqual(badge, 1, "unknown legacy outcome without incarnation must not poison future ask");
    host.ask(ask);
    assert.strictEqual(notices, 1);
    host.resolved({ chatId: ask.chatId, toolCallId: ask.toolCallId, resolution: "cancelled" });
    assert.strictEqual(badge, 0, "known legacy correlated native outcome clears");
    host.ask(ask);
    assert.strictEqual(notices, 1, "legacy replay remains tombstoned");
    const newer = { ...ask, toolCallId: "future", askedAtMs: 10 };
    host.resolved({ ...newer, resolution: "answered" });
    host.ask(newer);
    assert.strictEqual(badge, 0, "timestamped resolution observed before ask suppresses replay");
    host.dispose();
    host.bind({ projectRoot: "", endpointIdentity: "http://localhost:8488" });
    host.ask(ask);
    assert.strictEqual(badge, 0, "disposed host cannot bind or resurrect");
}

runLegacyDecisionNotificationTests();

void runHostDecisionNotificationTests().catch(error => {
    console.error(error);
    process.exitCode = 1;
});


async function runRuntimeVersionsPayloadTests() {
    const loader = require("module");
    const originalLoad = loader._load;
    const modulePath = require.resolve("./sidebar");
    const previous = require.cache[modulePath];
    const originalBlob = global.rust_binary_blob;
    const versions = {
        plugin: { state: "observed", version: "8.7.0", source: "plugin" },
        requestedVersion: "8.7.0",
        selected: { state: "observed", version: "8.8.0", source: "local" },
        installed: null, staged: null,
        daemon: { state: "observed", version: "8.9.0", source: "daemon" },
        worker: { state: "observed", version: "9.0.0", commit: "worker-commit", source: "worker" },
    };
    try {
        loader._load = function(request: string, parent: { filename?: string }, isMain: boolean) {
            if (parent?.filename === modulePath) {
                if (request === "vscode") { return { workspace: { getConfiguration: () => ({ get: () => undefined }) } }; }
                if (request === "./getKeybindings") { return { getKeyBindingForChat: async () => "" }; }
                if (request.startsWith("./") && !["./backendStatus", "./sidebarConfig", "./runtimeVersions", "./sidebarPaths"].includes(request)) { return {}; }
            }
            return originalLoad.call(this, request, parent, isMain);
        };
        delete require.cache[modulePath];
        const { PanelWebview } = require("./sidebar");
        const panel = Object.create(PanelWebview.prototype);
        panel.bindDecisionScope = () => undefined;
        panel.getActiveWorkspace = () => "repo";
        panel.getCurrentProjectInfo = () => ({ name: "repo" });
        panel.getColorTheme = () => "dark";
        panel.getActiveFileInfo = () => null;
        panel.getSnippetFromEditor = () => null;
        let payload: unknown;
        panel.postMessageToChat = (message: { payload: unknown }) => { payload = message.payload; };
        global.rust_binary_blob = {
            runtime_versions: () => JSON.parse(JSON.stringify(versions)),
            backend_status: () => "ready", connection_issue: () => null,
            get_port: () => 8488, daemon_control_port: () => 8488,
            rust_url: () => "http://localhost:8488/p/test/", browser_url: () => "http://localhost:8488/p/test/",
        } as unknown as typeof global.rust_binary_blob;
        const initial = await panel.createInitialState();
        panel.handleSettingsChange();
        assert.deepStrictEqual(initial.config.runtimeVersions, versions);
        assert.deepStrictEqual((payload as { runtimeVersions: unknown }).runtimeVersions, versions);
        versions.selected.version = "unknown";
        versions.daemon = null as unknown as typeof versions.daemon;
        versions.worker = null as unknown as typeof versions.worker;
        panel.handleSettingsChange();
        assert.strictEqual((payload as { runtimeVersions: typeof versions }).runtimeVersions.worker, null, "updates carry full clearing snapshot");
    } finally {
        loader._load = originalLoad;
        global.rust_binary_blob = originalBlob;
        if (previous) { require.cache[modulePath] = previous; } else { delete require.cache[modulePath]; }
    }
}

void runRuntimeVersionsPayloadTests().catch(error => {
    console.error(error);
    process.exitCode = 1;
});

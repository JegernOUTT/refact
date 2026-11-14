/* eslint-disable @typescript-eslint/naming-convention */
import * as vscode from 'vscode';
import * as fetchH2 from 'fetch-h2';
import * as fetchAPI from "./fetchAPI";
import * as lspClient from 'vscode-languageclient/node';
import * as net from 'net';
import * as path from 'path';
import { register_commands } from './rconsoleCommands';
import { QuickActionProvider } from './quickProvider';
import * as os from 'os';
import * as refactBinary from './refactBinaryResolver';
import * as refactDaemon from './refactDaemon';
import { registerSharedRefactPath } from './refactPathRegistration';
import { backendReadyForStatus, connectionIssueDescription, WorkspaceRequiredError, type RefactConnectionIssue, type RefactBackendConnectionStatus } from './backendStatus';
import {
    lspClientReadyTimeoutMs,
    lspClientStopTimeoutMs,
    lspSocketCloseAction,
    lspSocketConnectTimeoutMs,
    attachStateForDaemonOpenProject,
    browserUrlForBackendStatus,
    connectionIssueForAttachError,
    shouldRunLifecycleGeneration,
} from './launchRustLifecycle';
import { WorkerHealthProbe } from './launchRustHealth';
import { emptyRuntimeVersions, lastObserved, observedVersion, workerBuildObservation, type RuntimeVersions } from './runtimeVersions';

const DEBUG_HTTP_PORT = 8001;
const DEBUG_LSP_PORT = 8002;

type ProjectProxyRequestInit = Omit<Partial<fetchH2.RequestInit>, "headers"> & {
    headers?: Record<string, string>;
};

type OpenPrimaryProjectResult = {
    response: refactDaemon.OpenProjectResponse;
    lspPort: number;
};

export class RustBinaryBlob {
    public asset_path: string;
    public binary_cache_path: string;
    public port: number = 0;
    public project_id: string = "";
    public lsp_port: number = 0;
    private daemon_auth_token: string = "";
    public lsp_disposable: vscode.Disposable | undefined = undefined;
    public lsp_client: lspClient.LanguageClient | undefined = undefined;
    public lsp_socket: net.Socket | undefined = undefined;
    public lsp_client_options: lspClient.LanguageClientOptions;
    private lifecycleQueue: Promise<void> = Promise.resolve();
    private lifecycleGeneration: number = 0;
    private reconnectGeneration: number | undefined = undefined;
    private reconnectTimer: NodeJS.Timeout | undefined = undefined;
    private reconnectAttempts: number = 0;
    private openedProjects: Map<string, refactDaemon.OpenProjectResponse> = new Map();
    private attachState: RefactBackendConnectionStatus = "connecting";
    private connectionIssue: RefactConnectionIssue | null = null;
    private attachStage: RefactConnectionIssue["stage"] = "engine_resolution";
    private controlPort: number = refactDaemon.DEFAULT_DAEMON_PORT;
    private workerHealthProbe: WorkerHealthProbe;
    private runtimeVersions: RuntimeVersions;
    private warned_engine_fallbacks: Set<string> = new Set();

    constructor(asset_path: string, binary_cache_path?: string) {
        this.asset_path = asset_path;
        this.runtimeVersions = emptyRuntimeVersions(this.plugin_metadata_version());
        this.binary_cache_path = binary_cache_path ?? path.join(asset_path, "refact-bin");
        this.workerHealthProbe = new WorkerHealthProbe({
            probe: () => this.probe_attached_worker(),
            recover: () => this.recover_unhealthy_worker(),
        });
        this.lsp_client_options = {
            documentSelector: [{ scheme: 'file', language: '*' }],
            diagnosticCollectionName: 'RUST LSP',
            progressOnInitialization: true,
            traceOutputChannel: vscode.window.createOutputChannel('RUST LSP'),
            revealOutputChannelOn: lspClient.RevealOutputChannelOn.Error,
        };
    }

    public x_debug(): number {
        let xdebug = vscode.workspace.getConfiguration().get("refactai.xDebug");
        if (xdebug === undefined || xdebug === null || xdebug === 0 || xdebug === "0" || xdebug === false || xdebug === "false") {
            return 0;
        }
        return 1;
    }

    public get_port(): number {
        if (this.connectionIssue) {
            return 0;
        }
        if (this.x_debug()) {
            return DEBUG_HTTP_PORT;
        }
        return this.port;
    }

    public rust_url(): string {
        if (this.connectionIssue) {
            return "";
        }
        if (this.x_debug()) {
            return `http2://127.0.0.1:${DEBUG_HTTP_PORT}/`;
        }
        if (!this.port || !this.project_id) {
            return "";
        }
        return refactDaemon.projectProxyBaseUrl(this.port, this.project_id);
    }

    public backend_status(): RefactBackendConnectionStatus {
        return this.attachState;
    }

    public backend_ready(): boolean {
        return backendReadyForStatus(this.attachState) && !this.connectionIssue;
    }

    public connection_issue(): RefactConnectionIssue | null {
        return this.connectionIssue;
    }

    public runtime_versions(): RuntimeVersions {
        return JSON.parse(JSON.stringify(this.runtimeVersions)) as RuntimeVersions;
    }

    private publish_runtime_versions(): void {
        global.side_panel?.handleSettingsChange();
        global.open_chat_tabs?.forEach(tab => tab.handleSettingsChange());
    }

    private stale_runtime_versions(): void {
        this.runtimeVersions.daemon = lastObserved(this.runtimeVersions.daemon);
        this.runtimeVersions.worker = lastObserved(this.runtimeVersions.worker);
    }

    public daemon_control_port(): number {
        return refactDaemon.normalizeDaemonSpawnPort(this.controlPort) ?? refactDaemon.DEFAULT_DAEMON_PORT;
    }

    public workspace_folders_changed(): void {
        if (this.connectionIssue?.code !== "workspace_required" || !this.workspace_roots().length) {
            return;
        }
        // Clear synchronously: multiple webviews observe the same folder event.
        this.set_attach_state("connecting");
        void this.settings_changed();
    }

    public set_backend_status_for_test(status: RefactBackendConnectionStatus) {
        this.set_attach_state(status);
    }

    private set_attach_state(status: RefactBackendConnectionStatus, issue: RefactConnectionIssue | null = null) {
        if (this.attachState === status && this.connectionIssue === issue) {
            return;
        }
        this.attachState = status;
        this.connectionIssue = issue;
        global.side_panel?.handleSettingsChange();
        global.open_chat_tabs?.forEach(tab => tab.handleSettingsChange());
        global.status_bar?.choose_color();
    }

    private is_current_generation(generation: number): boolean {
        return shouldRunLifecycleGeneration(generation, this.lifecycleGeneration);
    }

    private dispose_lsp_resources(
        socket: net.Socket,
        client?: lspClient.LanguageClient,
        disposable?: vscode.Disposable,
    ): void {
        const currentSocket = this.lsp_socket === socket;
        if (disposable) {
            disposable.dispose();
            if (this.lsp_disposable === disposable) {
                this.lsp_disposable = undefined;
            }
        } else if (currentSocket && this.lsp_disposable) {
            this.lsp_disposable.dispose();
            this.lsp_disposable = undefined;
        }
        if (client) {
            if (this.lsp_client === client) {
                this.lsp_client = undefined;
            }
        } else if (currentSocket) {
            this.lsp_client = undefined;
        }
        if (currentSocket) {
            this.lsp_socket = undefined;
        }
        if (!socket.destroyed) {
            socket.destroy();
        }
    }

    private mark_lsp_disconnected(): void {
        this.workerHealthProbe.stop();
        this.stale_runtime_versions();
        global.have_caps = false;
        this.set_attach_state("connecting");
    }

    private mark_lsp_ready(): void {
        this.set_attach_state("ready");
        this.reconnectAttempts = 0;
        if (!this.x_debug()) {
            this.workerHealthProbe.start();
        }
    }

    private mark_worker_unhealthy(): void {
        this.lsp_dispose();
        this.stale_runtime_versions();
        global.have_caps = false;
        this.set_attach_state("failed", { code: "engine_start_failed", stage: "health_check" });
    }

    private async probe_attached_worker(): Promise<boolean> {
        if (this.x_debug() || this.attachState !== "ready" || !this.port || !this.project_id) {
            return true;
        }
        const generation = this.lifecycleGeneration;
        const endpoint = this.rust_url();
        const roots = JSON.stringify(this.workspace_roots());
        const current = () => roots === JSON.stringify(this.workspace_roots()) && this.is_current_generation(generation) && this.attachState === "ready" && endpoint === this.rust_url();
        const staleWorker = () => {
            if (current() && this.runtimeVersions.worker?.state === "observed") {
                this.runtimeVersions.worker = lastObserved(this.runtimeVersions.worker);
                this.publish_runtime_versions();
            }
        };
        try {
            const resp = await this.fetch_project_proxy_once("/v1/build_info", {
                method: "GET",
                redirect: "follow",
                cache: "no-cache",
                referrer: "no-referrer",
            }, { timeout: 5000 });
            if (!current()) { return true; }
            if (resp.status !== 200) {
                staleWorker();
                console.log(["worker health http status", resp.status]);
                return false;
            }
            try {
                const info = await resp.json();
                if (!current()) { return true; }
                const observation = workerBuildObservation(info);
                if (!observation) { staleWorker(); }
                if (observation) {
                    const previous = this.runtimeVersions.worker;
                    this.runtimeVersions.worker = observation;
                    if (previous?.state !== observation.state || previous?.version !== observation.version || previous?.commit !== observation.commit) {
                        this.publish_runtime_versions();
                    }
                }
            } catch (error) {
                staleWorker();
                console.log(["worker build info parse error", error]);
            }
            return true;
        } catch (error) {
            staleWorker();
            console.log(["worker health error", error]);
            return !current();
        }
    }

    private recover_unhealthy_worker(): void {
        if (this.x_debug() || !this.is_current_generation(this.lifecycleGeneration)) {
            return;
        }
        console.log("RUST worker health failed repeatedly; scheduling daemon reconnect");
        this.mark_worker_unhealthy();
        this.reconnectGeneration = this.lifecycleGeneration;
        this.schedule_lsp_reconnect(this.lifecycleGeneration);
    }

    public browser_url(): string {
        const configuredHost = vscode.workspace.getConfiguration().get<string>("refactai.browserHost")?.trim();
        return browserUrlForBackendStatus({
            status: this.attachState,
            debug: Boolean(this.x_debug()),
            debugHttpPort: DEBUG_HTTP_PORT,
            port: this.port,
            projectId: this.project_id,
            configuredHost,
            authToken: this.daemon_auth_token,
        });
    }

    public attemping_to_reach(): string {
        if (this.x_debug()) {
            return `debug rust binary on ports ${DEBUG_HTTP_PORT} and ${DEBUG_LSP_PORT}`;
        }
        if (this.project_id) {
            return `Refact daemon project ${this.project_id} on port ${this.daemon_port()}`;
        }
        return `Refact daemon on port ${this.daemon_port()}`;
    }

    public async settings_changed() {
        return this.enqueueLifecycle(async (generation) => this.settings_changed_serialized(generation));
    }

    private enqueueLifecycle(operation: (generation: number) => Promise<void>, automatic = false): Promise<void> {
        const generation = ++this.lifecycleGeneration;
        if (automatic) {
            this.stale_runtime_versions();
        } else {
            this.runtimeVersions = emptyRuntimeVersions(this.plugin_metadata_version());
        }
        const run = this.lifecycleQueue.catch(() => undefined).then(() => {
            if (!this.is_current_generation(generation)) {
                return undefined;
            }
            return operation(generation);
        });
        this.lifecycleQueue = run.catch(() => undefined);
        return run;
    }

    private async settings_changed_serialized(generation: number, automatic = false) {
        this.workerHealthProbe.stop();
        const heldIssue = automatic && this.connectionIssue?.code === "workspace_busy" ? this.connectionIssue : null;
        this.attachStage = "engine_resolution";
        if (!automatic) {
            this.controlPort = this.daemon_port();
        }
        this.set_attach_state("connecting", heldIssue);
        global.status_bar?.set_socket_error(false, "");
        try {
            if (this.x_debug()) {
                await this.attach_debug_serialized(generation);
            } else {
                await this.attach_daemon_serialized(generation, automatic, heldIssue);
            }
        } catch (error) {
            if (!this.is_current_generation(generation)) {
                return;
            }
            console.log(["Refact attach failed", error]);
            global.have_caps = false;
            this.reset_daemon_state();
            const issue = connectionIssueForAttachError(error, this.attachStage) ?? { code: "engine_start_failed" as const };
            this.stale_runtime_versions();
            this.set_attach_state("failed", issue);
            global.status_bar.set_socket_error(true, connectionIssueDescription(issue));
            if ((issue.code === "workspace_busy" || issue.stage === "lsp_attach" || issue.stage === "health_check") && !this.x_debug()) {
                this.reconnectGeneration = generation;
                this.schedule_lsp_reconnect(generation, issue.code === "workspace_busy" ? 5000 : 0);
            }
            if (issue?.code === "workspace_required") {
                this.workspace_folders_changed();
            }
        } finally {
            if (this.is_current_generation(generation)) {
                global.side_panel?.handleSettingsChange();
            }
        }
    }

    public async launch() {
        return this.enqueueLifecycle(async (generation) => this.launch_serialized(generation));
    }

    private async launch_serialized(generation: number) {
        await this.settings_changed_serialized(generation);
    }

    private async attach_debug_serialized(generation: number) {
        this.attachStage = "lsp_attach";
        this.clearReconnectTimer();
        await this.stop_lsp(generation);
        if (!this.is_current_generation(generation)) {
            return;
        }
        this.port = DEBUG_HTTP_PORT;
        this.lsp_port = DEBUG_LSP_PORT;
        this.project_id = "";
        this.daemon_auth_token = "";
        this.openedProjects.clear();
        console.log(`RUST debug is set, don't start the rust binary. Will attempt HTTP port ${DEBUG_HTTP_PORT}, LSP port ${DEBUG_LSP_PORT}`);
        console.log("Also, will try to read caps. If that fails, things like lists of available models will be empty.");
        await this.start_lsp_socket(generation);
    }

    private async attach_daemon_serialized(generation: number, automatic = false, heldIssue: RefactConnectionIssue | null = null) {
        this.clearReconnectTimer();
        await this.stop_lsp(generation);
        if (!this.is_current_generation(generation)) {
            return;
        }

        const daemonPort = this.daemon_port();
        const explicitDaemonPort = this.explicit_daemon_port();
        const roots = this.workspace_roots();
        const rootSelection = refactDaemon.selectPrimaryWorkspaceRoot(roots);
        if (!rootSelection.primary) {
            this.reset_daemon_state();
            throw new WorkspaceRequiredError();
        }
        this.warn_about_ignored_roots(rootSelection);

        const pluginVersion = this.plugin_version();
        this.attachStage = "engine_resolution";
        const localResolved = await refactBinary.resolveLocalRefactBinaryDetailedOrNull({
            explicitPath: vscode.workspace.getConfiguration().get<string>("refactai.binaryPath")?.trim(),
            bundledDir: this.asset_path,
            minVersion: pluginVersion,
            pinnedVersion: pluginVersion,
            cacheDir: this.binary_cache_path,
        });
        if (!this.is_current_generation(generation)) { return; }
        this.runtimeVersions.requestedVersion = pluginVersion;
        this.runtimeVersions.selected = localResolved ? observedVersion("local", localResolved.version) : null;
        const expectedExecutableSha256 = localResolved ? await refactDaemon.sha256OfFile(localResolved.binPath) : undefined;
        if (!this.is_current_generation(generation)) { return; }
        const expectedEngineVersion = localResolved?.version ?? pluginVersion;
        let daemon = await refactDaemon.findExistingDaemon({ port: daemonPort, pluginVersion: expectedEngineVersion, expectedExecutableSha256 });
        if (!this.is_current_generation(generation)) {
            return;
        }
        this.set_attach_state(attachStateForDaemonOpenProject(), heldIssue);
        if (daemon) {
            const sharedResolved = await refactBinary.resolveCompatibleSharedRefactBinaryOrNull(os.homedir(), pluginVersion);
            if (!this.is_current_generation(generation)) { return; }
            if (sharedResolved) {
                this.maybe_register_shared_refact_path(sharedResolved.binPath);
            }
        }
        if (!daemon) {
            // Background recovery must not download binaries or replace another IDE's daemon.
            // Only an explicit launch/settings action may run ensureDaemon's upgrade path.
            if (automatic) {
                this.attachStage = "daemon_start";
                throw new Error("Refact daemon is unavailable or incompatible; retry explicitly.");
            }
            const configuredBinary = vscode.workspace.getConfiguration().get<string>("refactai.binaryPath")?.trim();
            const resolved = await refactBinary.resolveRefactBinaryDetailed({
                explicitPath: configuredBinary,
                bundledDir: this.asset_path,
                minVersion: pluginVersion,
                pinnedVersion: pluginVersion,
                cacheDir: this.binary_cache_path,
                onDownloadStart: () => {
                    if (this.is_current_generation(generation)) {
                        this.set_attach_state("installing");
                    }
                },
                onFallbackVersion: (pinned, chosen) => {
                    if (this.is_current_generation(generation)) { this.warn_engine_version_fallback(pinned, chosen); }
                },
            });
            if (!this.is_current_generation(generation)) {
                return;
            }
            this.runtimeVersions.selected = observedVersion("local", resolved.version);
            this.maybe_register_shared_refact_path(resolved.binPath);
            this.attachStage = "daemon_start";
            daemon = await refactDaemon.ensureDaemon(resolved.binPath, {
                port: daemonPort,
                pluginVersion: resolved.version ?? pluginVersion,
            }, explicitDaemonPort);
        }
        if (!this.is_current_generation(generation)) {
            return;
        }

        const daemonPortForGeneration = daemon.port;
        this.runtimeVersions.daemon = typeof daemon.version === "string" && daemon.version.trim() ? observedVersion("daemon", daemon.version) : null;
        if (this.port !== daemonPortForGeneration) { this.runtimeVersions.worker = null; }
        this.controlPort = daemonPortForGeneration;
        this.attachStage = "project_open";
        const daemonAuthTokenForGeneration = this.auth_token_for_port(daemonPortForGeneration, daemon.authToken);
        const projectResponse = await this.open_primary_project_response(
            rootSelection.primary,
            daemonPortForGeneration,
            daemonAuthTokenForGeneration,
        );
        if (!this.is_current_generation(generation)) {
            return;
        }
        this.port = daemonPortForGeneration;
        this.daemon_auth_token = this.auth_token_for_port(daemonPortForGeneration, daemonAuthTokenForGeneration);
        this.openedProjects.clear();
        this.openedProjects.set(rootSelection.primary, projectResponse.response);
        if (this.project_id !== projectResponse.response.project_id) { this.runtimeVersions.worker = null; }
        this.project_id = projectResponse.response.project_id;
        this.lsp_port = projectResponse.lspPort;
        this.attachStage = "lsp_attach";
        await this.start_lsp_socket(generation);
    }

    public async stop_lsp(generation: number = this.lifecycleGeneration) {
        if (this.is_current_generation(generation)) {
            this.workerHealthProbe.stop();
            this.reconnectGeneration = undefined;
            this.clearReconnectTimer();
        }
        const my_lsp_client_copy = this.lsp_client;
        if (my_lsp_client_copy) {
            console.log("RUST STOP");
            const ts = Date.now();
            try {
                await Promise.race([
                    my_lsp_client_copy.stop(),
                    new Promise<void>(resolve => setTimeout(resolve, lspClientStopTimeoutMs)),
                ]);
                console.log(`RUST /STOP completed in ${Date.now() - ts}ms`);
            } catch (e) {
                console.log(`RUST STOP ERROR e=${e}`);
            } finally {
                console.log("RUST STOP FINALLY");
            }
        }
        if (this.is_current_generation(generation)) {
            this.lsp_dispose();
        }
    }

    public lsp_dispose() {
        this.workerHealthProbe.stop();
        if (this.lsp_disposable) {
            this.lsp_disposable.dispose();
            this.lsp_disposable = undefined;
        }
        const socket = this.lsp_socket;
        this.lsp_client = undefined;
        this.lsp_socket = undefined;
        if (socket && !socket.destroyed) {
            socket.destroy();
        }
    }

    public async terminate() {
        this.workerHealthProbe.dispose();
        this.lifecycleGeneration++;
        this.runtimeVersions = emptyRuntimeVersions(this.plugin_metadata_version());
        this.reconnectGeneration = undefined;
        this.clearReconnectTimer();
        await this.stop_lsp(this.lifecycleGeneration);
        await refactDaemon.detach();
        await fetchH2.disconnectAll();
        global.have_caps = false;
        global.status_bar.choose_color();
        this.reset_daemon_state();
        this.set_attach_state("connecting");
    }

    public async fetch_project_proxy(
        addthis: string,
        init: ProjectProxyRequestInit,
        fetchInit?: Partial<fetchH2.FetchInit>,
    ): Promise<fetchH2.Response> {
        if (this.x_debug()) {
            return this.fetch_project_proxy_once(addthis, init, fetchInit);
        }
        return refactDaemon.projectProxyFetchWithRetry(
            () => this.fetch_project_proxy_once(addthis, init, fetchInit),
            async () => {
                console.log(["project proxy unavailable, reopening project", addthis]);
                await this.reopen_primary_project();
            },
            response => response.status,
        );
    }

    public project_proxy_url(addthis: string): string {
        let url = this.rust_url();
        if (!url) {
            return "";
        }
        while (url.endsWith("/")) {
            url = url.slice(0, -1);
        }
        const suffix = addthis.startsWith("/") ? addthis : `/${addthis}`;
        return url + suffix;
    }

    private async fetch_project_proxy_once(
        addthis: string,
        init: ProjectProxyRequestInit,
        fetchInit?: Partial<fetchH2.FetchInit>,
    ): Promise<fetchH2.Response> {
        const url = this.project_proxy_url(addthis);
        if (!url) {
            return Promise.reject("No rust binary working");
        }
        const request = new fetchH2.Request(url, {
            ...init,
            headers: refactDaemon.daemonRequestHeaders(this.x_debug() ? "" : this.daemon_auth_token, init.headers),
        });
        return fetchH2.fetch(request, fetchInit);
    }

    private async reopen_primary_project(): Promise<void> {
        const rootSelection = refactDaemon.selectPrimaryWorkspaceRoot(this.workspace_roots());
        if (!rootSelection.primary) {
            this.reset_daemon_state();
            throw new WorkspaceRequiredError();
        }
        this.warn_about_ignored_roots(rootSelection);
        await this.open_primary_project(rootSelection.primary);
    }

    private async open_primary_project_response(
        root: string,
        port: number,
        authToken: string,
    ): Promise<OpenPrimaryProjectResult> {
        const response = await refactDaemon.openProject(root, {
            port,
            authToken,
            clientKind: "vscode",
            settings: this.project_settings(),
        });
        const lspPort = response.worker?.lsp_port;
        if (!lspPort) {
            throw new Error(`Refact daemon opened project ${response.project_id} without an LSP port.`);
        }
        return { response, lspPort };
    }

    private async open_primary_project(root: string): Promise<void> {
        if (!this.port) {
            throw new Error("Refact daemon port is not available.");
        }
        const generation = this.lifecycleGeneration;
        const port = this.port;
        const projectResponse = await this.open_primary_project_response(root, port, this.daemon_auth_token);
        if (!this.is_current_generation(generation) || this.port !== port) { return; }
        this.daemon_auth_token = this.auth_token_for_port(this.port, this.daemon_auth_token);
        this.openedProjects.clear();
        this.openedProjects.set(root, projectResponse.response);
        if (this.project_id !== projectResponse.response.project_id) { this.runtimeVersions.worker = null; }
        this.project_id = projectResponse.response.project_id;
        this.lsp_port = projectResponse.lspPort;
    }

    private auth_token_for_port(port: number, fallback?: string | null): string {
        const endpoint = refactDaemon.daemonEndpoints({ port }).find(candidate => candidate.port === port && candidate.authToken);
        return endpoint?.authToken ?? fallback ?? "";
    }

    private warn_about_ignored_roots(selection: refactDaemon.PrimaryWorkspaceRootSelection): void {
        if (!selection.warning) {
            return;
        }
        console.log(selection.warning);
        void vscode.window.showWarningMessage(selection.warning);
    }

    private reset_daemon_state(): void {
        this.project_id = "";
        this.lsp_port = 0;
        this.port = 0;
        this.daemon_auth_token = "";
        this.openedProjects.clear();
    }

    public async read_caps() {
        try {
            const resp = await this.fetch_project_proxy("/v1/caps", {
                method: "GET",
                redirect: "follow",
                cache: "no-cache",
                referrer: "no-referrer"
            }, { timeout: 5000 });
            if (resp.status !== 200) {
                console.log(["read_caps http status", resp.status]);
                return Promise.reject("read_caps bad status");
            }
            let json = await resp.json();
            console.log(["successful read_caps", json]);
            global.chat_models = Object.keys(json["chat_models"]);
            global.chat_default_model = json["chat_default_model"] || "";
            global.have_caps = true;
            global.status_bar.set_socket_error(false, "");
        } catch (e) {
            global.chat_models = [];
            global.have_caps = false;
            console.log(["read_caps:", e]);
        }
        global.status_bar.choose_color();
        fetchAPI.maybe_show_rag_status();
        let current_editor = vscode.window.activeTextEditor;
        if (current_editor) {
            fetchAPI.lsp_set_active_document(current_editor);
        }

        const promptCustomization = await fetchAPI.get_prompt_customization().catch(error => {
            console.log(["get_prompt_customization", error]);
            return undefined;
        });
        if (promptCustomization && promptCustomization.toolbox_commands) {
            await QuickActionProvider.updateActions(promptCustomization.toolbox_commands as Record<string, ToolboxCommand>);
        }
    }

    public async ping() {
        try {
            let resp = await this.fetch_project_proxy("/v1/ping", {
                method: "GET",
                redirect: "follow",
                cache: "no-cache",
                referrer: "no-referrer",
            }, { timeout: 5000 });
            if (resp.status !== 200) {
                console.log(["ping http status", resp.status]);
                return false;
            }
            return true;
        } catch (e) {
            console.log(["ping error:", e]);
        }
        return false;
    }

    private async wait_lsp_client_ready(client: lspClient.LanguageClient): Promise<void> {
        let readyTimer: NodeJS.Timeout | undefined;
        try {
            await Promise.race([
                client.onReady(),
                new Promise<void>((_resolve, reject) => {
                    readyTimer = setTimeout(() => {
                        reject(new Error(`RUST LSP client onReady timed out after ${lspClientReadyTimeoutMs}ms`));
                    }, lspClientReadyTimeoutMs);
                }),
            ]);
        } finally {
            if (readyTimer) {
                clearTimeout(readyTimer);
            }
        }
    }

    public async start_lsp_socket(generation: number = this.lifecycleGeneration) {
        if (!this.is_current_generation(generation)) {
            return;
        }
        const lspPort = this.x_debug() ? DEBUG_LSP_PORT : this.lsp_port;
        if (!lspPort) {
            throw new Error("Refact LSP port is not available.");
        }
        console.log(`RUST start_lsp_socket ${lspPort}`);
        this.reconnectGeneration = this.x_debug() ? undefined : generation;

        await new Promise<void>((resolve, reject) => {
            let settled = false;
            let clientReady = false;
            let connectTimer: NodeJS.Timeout | undefined;
            const finish = (error?: Error) => {
                if (settled) {
                    return;
                }
                settled = true;
                if (connectTimer) {
                    clearTimeout(connectTimer);
                    connectTimer = undefined;
                }
                if (error) {
                    reject(error);
                    return;
                }
                resolve();
            };
            const fail_or_reconnect = (socket: net.Socket, error: Error) => {
                this.dispose_lsp_resources(socket);
                if (!this.is_current_generation(generation)) {
                    finish();
                    return;
                }
                this.mark_lsp_disconnected();
                if (this.x_debug()) {
                    finish(error);
                    return;
                }
                this.set_attach_state("failed", { code: "engine_start_failed", stage: "lsp_attach" });
                this.schedule_lsp_reconnect(generation);
                finish();
            };
            const socket = new net.Socket();
            this.lsp_socket = socket;
            socket.on('error', (error) => {
                console.log("RUST socket error");
                console.log(error);
                console.log("RUST /error");
            });
            socket.on('close', () => {
                console.log("RUST socket closed");
                const closedBeforeReady = !clientReady;
                const action = lspSocketCloseAction({
                    generation,
                    currentGeneration: this.lifecycleGeneration,
                    reconnectGeneration: this.reconnectGeneration,
                    socketIsCurrent: this.lsp_socket === socket,
                    debug: Boolean(this.x_debug()),
                });
                if (action === "ignore") {
                    finish();
                    return;
                }
                this.dispose_lsp_resources(socket);
                this.mark_lsp_disconnected();
                if (action === "reconnect") {
                    if (closedBeforeReady) {
                        this.set_attach_state("failed", { code: "engine_start_failed", stage: "lsp_attach" });
                    }
                    this.schedule_lsp_reconnect(generation);
                }
                if (action === "disconnect" && this.x_debug() && closedBeforeReady) {
                    finish(new Error("RUST LSP socket closed before ready"));
                    return;
                }
                finish();
            });
            socket.on('connect', async () => {
                if (connectTimer) {
                    clearTimeout(connectTimer);
                    connectTimer = undefined;
                }
                if (!this.is_current_generation(generation)) {
                    socket.destroy();
                    finish();
                    return;
                }
                console.log("RUST LSP socket connected");
                const client = new lspClient.LanguageClient(
                    'Custom rust LSP server',
                    async () => {
                        if (this.lsp_socket !== socket || !this.is_current_generation(generation)) {
                            return Promise.reject("RUST LSP socket was superseded");
                        }
                        return Promise.resolve({
                            reader: socket,
                            writer: socket,
                        });
                    },
                    this.lsp_client_options
                );
                this.lsp_client = client;
                const disposable = client.start();
                this.lsp_disposable = disposable;
                console.log(`RUST START`);
                try {
                    await this.wait_lsp_client_ready(client);
                    if (!this.is_current_generation(generation) || this.lsp_socket !== socket) {
                        this.dispose_lsp_resources(socket, client, disposable);
                        finish();
                        return;
                    }
                    clientReady = true;
                    this.mark_lsp_ready();
                    void this.probe_attached_worker();
                    console.log(`RUST /START`);
                    await this.read_caps();
                    if (!this.is_current_generation(generation)) {
                        finish();
                        return;
                    }
                    if (this.lsp_socket !== socket) {
                        this.mark_lsp_disconnected();
                        finish();
                        return;
                    }
                    await this.fetch_toolbox_config().catch(error => console.log(["fetch_toolbox_config", error]));
                } catch (e) {
                    const error = e instanceof Error ? e : new Error(String(e));
                    console.log(`RUST START PROBLEM e=${error.message}`);
                    this.dispose_lsp_resources(socket, client, disposable);
                    if (!this.is_current_generation(generation)) {
                        finish();
                        return;
                    }
                    this.mark_lsp_disconnected();
                    if (this.x_debug()) {
                        finish(error);
                        return;
                    }
                    this.set_attach_state("failed", { code: "engine_start_failed", stage: "lsp_attach" });
                    this.schedule_lsp_reconnect(generation);
                }
                finish();
            });
            connectTimer = setTimeout(() => {
                if (!settled) {
                    console.log("RUST LSP socket connect timeout");
                    fail_or_reconnect(socket, new Error(`RUST LSP socket connect timed out after ${lspSocketConnectTimeoutMs}ms`));
                }
            }, lspSocketConnectTimeoutMs);
            socket.connect(lspPort, "127.0.0.1");
        });
    }

    async rag_status() {
        try {
            let resp = await this.fetch_project_proxy("/v1/rag-status", {
                method: "GET",
                redirect: "follow",
                cache: "no-cache",
                referrer: "no-referrer",
            }, { timeout: 5000 });
            if (resp.status !== 200) {
                console.log(["rag status http status", resp.status]);
                return Promise.reject("rag status bad status");
            }
            let rag_status = await resp.json();
            return rag_status;
        } catch (e) {
            console.log(["rag status error:", e]);
        }
        return false;
    }

    async fetch_toolbox_config(): Promise<ToolboxConfig> {
        const response = await this.fetch_project_proxy("/v1/customization", { method: "GET" }, { timeout: 5000 });

        if (!response.ok) {
            console.log([
                "fetch_toolbox_config: Error fetching toolbox config",
                response.status,
                this.project_proxy_url("/v1/customization"),
            ]);
            return Promise.reject(
                `Error fetching toolbox config: [status: ${response.status}] [statusText: ${response.statusText}]`
            );
        }

        const json = await response.json() as ToolboxConfig;
        console.log(["success fetch_toolbox_config", json]);

        global.toolbox_config = json;
        await register_commands();
        return json;
    }

    private schedule_lsp_reconnect(generation: number, minimumDelay = 0) {
        if (this.reconnectTimer || !this.is_current_generation(generation)) {
            return;
        }
        const delay = Math.max(minimumDelay, Math.min(30000, 1000 * Math.pow(2, this.reconnectAttempts++)));
        console.log(`RUST scheduling daemon LSP reconnect in ${delay}ms`);
        this.reconnectTimer = setTimeout(() => {
            this.reconnectTimer = undefined;
            if (!this.is_current_generation(generation) || this.reconnectGeneration !== generation) {
                return;
            }
            void this.enqueueLifecycle(async (nextGeneration) => this.settings_changed_serialized(nextGeneration, true), true);
        }, delay);
    }

    private clearReconnectTimer() {
        if (this.reconnectTimer) {
            clearTimeout(this.reconnectTimer);
            this.reconnectTimer = undefined;
        }
    }

    private daemon_port(): number {
        const port = vscode.workspace.getConfiguration().get<number>("refactai.daemonPort");
        return refactDaemon.normalizeDaemonSpawnPort(port) ?? refactDaemon.DEFAULT_DAEMON_PORT;
    }

    private explicit_daemon_port(): number | undefined {
        const configuration = vscode.workspace.getConfiguration();
        const inspected = configuration.inspect<number>("refactai.daemonPort");
        if (!inspected || (inspected.globalValue === undefined
            && inspected.workspaceValue === undefined
            && inspected.workspaceFolderValue === undefined)) {
            return undefined;
        }
        return refactDaemon.normalizeDaemonSpawnPort(configuration.get<number>("refactai.daemonPort"));
    }

    private maybe_register_shared_refact_path(binPath: string): void {
        // Only register the canonical IDE-managed / shared binary
        // (~/.refact/bin/refact[.exe]). Never register explicit, bundled, or
        // arbitrary PATH binaries. Failures must never abort attach.
        const homeDir = os.homedir();
        if (!refactBinary.isSharedRefactBinaryPath(binPath, homeDir)) {
            return;
        }
        void registerSharedRefactPath({ homeDir })
            .then(result => {
                if (result.outcome === "warning") {
                    console.warn(`Refact PATH registration skipped: ${result.message ?? "unknown reason"}`);
                }
            })
            .catch(error => {
                console.warn(`Refact PATH registration failed: ${error instanceof Error ? error.message : String(error)}`);
            });
    }

    private plugin_version(): string {
        return this.plugin_metadata_version() ?? "0.0.0";
    }

    private plugin_metadata_version(): string | undefined {
        return vscode.extensions.getExtension("smallcloud.codify")?.packageJSON?.version;
    }

    private warn_engine_version_fallback(pinned: string, chosen: string): void {
        const key = `${pinned}->${chosen}`;
        if (this.warned_engine_fallbacks.has(key)) {
            return;
        }
        this.warned_engine_fallbacks.add(key);
        const message = `Refact engine v${pinned} is not published on GitHub releases; using the latest published engine v${chosen} instead.`;
        console.log(message);
        void vscode.window.showWarningMessage(message);
    }

    private workspace_roots(): string[] {
        return (vscode.workspace.workspaceFolders ?? [])
            .filter(folder => folder.uri.scheme === "file")
            .map(folder => folder.uri.fsPath);
    }

    private project_settings(): refactDaemon.ProjectSettings {
        const config = vscode.workspace.getConfiguration();
        return {
            ast: config.get<boolean>("refactai.ast") ?? true,
            vecdb: config.get<boolean>("refactai.vecdb") ?? true,
            codegraph: config.get<boolean>("refactai.codegraph") ?? true,
            ast_max_files: config.get<number>("refactai.astFileLimit") ?? 35000,
            vecdb_max_files: config.get<number>("refactai.vecdbFileLimit") ?? 15000,
            codegraph_max_files: config.get<number>("refactai.codegraphFileLimit") ?? 35000,
        };
    }
}

export type ChatMessageFromLsp = {
    role: string;
    content: string;
};

export type ToolboxCommand = {
    description: string;
    messages: ChatMessageFromLsp[];
    selection_needed: number[];
    selection_unwanted: boolean;
    insert_at_cursor: boolean;
};

export type SystemPrompt = {
    description: string;
    text: string;
};

export type ToolboxConfig = {
    system_prompts: Record<string, SystemPrompt>;
    toolbox_commands: Record<string, ToolboxCommand>;
};

/* eslint-disable @typescript-eslint/naming-convention */
import * as vscode from 'vscode';
import * as fetchH2 from 'fetch-h2';
import * as fetchAPI from "./fetchAPI";
import * as lspClient from 'vscode-languageclient/node';
import * as net from 'net';
import * as os from 'os';
import { register_commands } from './rconsoleCommands';
import { QuickActionProvider } from './quickProvider';
import * as refactDaemon from './refactDaemon';

const DEBUG_HTTP_PORT = 8001;
const DEBUG_LSP_PORT = 8002;

export class RustBinaryBlob {
    public asset_path: string;
    public port: number = 0;
    public project_id: string = "";
    public lsp_port: number = 0;
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

    constructor(asset_path: string) {
        this.asset_path = asset_path;
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
        if (this.x_debug()) {
            return DEBUG_HTTP_PORT;
        }
        return this.port;
    }

    public rust_url(): string {
        if (this.x_debug()) {
            return `http2://127.0.0.1:${DEBUG_HTTP_PORT}/`;
        }
        if (!this.port || !this.project_id) {
            return "";
        }
        return refactDaemon.projectProxyBaseUrl(this.port, this.project_id);
    }

    public browser_url(): string {
        if (this.x_debug()) {
            const configuredHost = vscode.workspace.getConfiguration().get<string>("refactai.browserHost")?.trim();
            const host = configuredHost && configuredHost !== "0.0.0.0" ? configuredHost : this.default_browser_host();
            return `http://${host}:${DEBUG_HTTP_PORT}/`;
        }
        if (!this.port || !this.project_id) {
            return "";
        }
        const configuredHost = vscode.workspace.getConfiguration().get<string>("refactai.browserHost")?.trim();
        const host = configuredHost && configuredHost !== "0.0.0.0" ? configuredHost : this.default_browser_host();
        return refactDaemon.browserProjectUrl(host, this.port, this.project_id);
    }

    private default_browser_host(): string {
        return this.default_mdns_host();
    }

    private default_mdns_host(): string {
        const hostname = os.hostname() as string;
        const label = hostname
            .toLowerCase()
            .replace(/[^a-z0-9-]/g, "-")
            .replace(/^-+|-+$/g, "");
        return label ? `${label}.local` : "refact.local";
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

    private enqueueLifecycle(operation: (generation: number) => Promise<void>): Promise<void> {
        const generation = ++this.lifecycleGeneration;
        const run = this.lifecycleQueue.catch(() => undefined).then(() => operation(generation));
        this.lifecycleQueue = run.catch(() => undefined);
        return run;
    }

    private async settings_changed_serialized(generation: number) {
        try {
            if (this.x_debug()) {
                await this.attach_debug_serialized(generation);
            } else {
                await this.attach_daemon_serialized(generation);
            }
        } catch (error) {
            console.log(["Refact attach failed", error]);
            global.have_caps = false;
            global.status_bar.set_socket_error(true, error instanceof Error ? error.message : String(error));
        } finally {
            global.side_panel?.handleSettingsChange();
        }
    }

    public async launch() {
        return this.enqueueLifecycle(async (generation) => this.launch_serialized(generation));
    }

    private async launch_serialized(generation: number) {
        await this.settings_changed_serialized(generation);
    }

    private async attach_debug_serialized(generation: number) {
        this.clearReconnectTimer();
        await this.stop_lsp(generation);
        if (generation !== this.lifecycleGeneration) {
            return;
        }
        this.port = DEBUG_HTTP_PORT;
        this.lsp_port = DEBUG_LSP_PORT;
        this.project_id = "";
        this.openedProjects.clear();
        console.log(`RUST debug is set, don't start the rust binary. Will attempt HTTP port ${DEBUG_HTTP_PORT}, LSP port ${DEBUG_LSP_PORT}`);
        console.log("Also, will try to read caps. If that fails, things like lists of available models will be empty.");
        await this.start_lsp_socket(generation);
    }

    private async attach_daemon_serialized(generation: number) {
        this.clearReconnectTimer();
        await this.stop_lsp(generation);
        if (generation !== this.lifecycleGeneration) {
            return;
        }

        const daemonPort = this.daemon_port();
        const roots = this.workspace_roots();
        if (roots.length === 0) {
            this.project_id = "";
            this.lsp_port = 0;
            throw new Error("Open a workspace folder before starting Refact.");
        }

        this.port = daemonPort;
        const binPath = refactDaemon.resolveBundledRefactPath(this.asset_path);
        const pluginVersion = this.plugin_version();
        await refactDaemon.ensureDaemon(binPath, { port: daemonPort, pluginVersion });
        if (generation !== this.lifecycleGeneration) {
            return;
        }

        this.openedProjects.clear();
        const settings = this.project_settings();
        let primary: refactDaemon.OpenProjectResponse | undefined;
        for (const root of roots) {
            const response = await refactDaemon.openProject(root, {
                port: daemonPort,
                clientKind: "vscode",
                settings,
            });
            this.openedProjects.set(root, response);
            if (!primary) {
                primary = response;
            }
        }

        if (!primary) {
            throw new Error("Refact daemon did not open a project.");
        }
        const lspPort = primary.worker?.lsp_port;
        if (!lspPort) {
            throw new Error(`Refact daemon opened project ${primary.project_id} without an LSP port.`);
        }
        this.project_id = primary.project_id;
        this.lsp_port = lspPort;
        await this.start_lsp_socket(generation);
    }

    public async stop_lsp(generation: number = this.lifecycleGeneration) {
        if (generation === this.lifecycleGeneration) {
            this.reconnectGeneration = undefined;
            this.clearReconnectTimer();
        }
        let my_lsp_client_copy = this.lsp_client;
        if (my_lsp_client_copy) {
            console.log("RUST STOP");
            let ts = Date.now();
            try {
                await Promise.race([
                    my_lsp_client_copy.stop(),
                    new Promise<void>(resolve => setTimeout(resolve, 5000)),
                ]);
                console.log(`RUST /STOP completed in ${Date.now() - ts}ms`);
            } catch (e) {
                console.log(`RUST STOP ERROR e=${e}`);
            } finally {
                console.log("RUST STOP FINALLY");
            }
        }
        if (generation === this.lifecycleGeneration) {
            this.lsp_dispose();
        }
    }

    public lsp_dispose() {
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
        return this.enqueueLifecycle(async (generation) => this.terminate_serialized(generation));
    }

    private async terminate_serialized(generation: number) {
        await this.stop_lsp(generation);
        if (generation !== this.lifecycleGeneration) {
            return;
        }
        await refactDaemon.detach();
        await fetchH2.disconnectAll();
        global.have_caps = false;
        global.status_bar.choose_color();
    }

    public async read_caps() {
        try {
            let url = this.rust_url();
            if (!url) {
                return Promise.reject("read_caps no rust binary working, very strange");
            }
            url += "v1/caps";
            let req = new fetchH2.Request(url, {
                method: "GET",
                redirect: "follow",
                cache: "no-cache",
                referrer: "no-referrer"
            });
            let resp = await fetchH2.fetch(req);
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
            let url = this.rust_url();
            if (!url) {
                return Promise.reject("ping no rust binary working, very strange");
            }
            url += "v1/ping";
            console.log([url]);
            let req = new fetchH2.Request(url, {
                method: "GET",
                redirect: "follow",
                cache: "no-cache",
                referrer: "no-referrer",
            });
            let resp = await fetchH2.fetch(req, { timeout: 5000 });
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

    public async start_lsp_socket(generation: number = this.lifecycleGeneration) {
        const lspPort = this.x_debug() ? DEBUG_LSP_PORT : this.lsp_port;
        if (!lspPort) {
            throw new Error("Refact LSP port is not available.");
        }
        console.log(`RUST start_lsp_socket ${lspPort}`);
        this.reconnectGeneration = this.x_debug() ? undefined : generation;

        await new Promise<void>((resolve) => {
            let settled = false;
            const finish = () => {
                if (!settled) {
                    settled = true;
                    resolve();
                }
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
                const shouldReconnect = this.reconnectGeneration === generation && generation === this.lifecycleGeneration && !this.x_debug();
                this.lsp_dispose();
                if (shouldReconnect) {
                    this.schedule_lsp_reconnect(generation);
                }
                finish();
            });
            socket.on('connect', async () => {
                if (generation !== this.lifecycleGeneration) {
                    socket.destroy();
                    finish();
                    return;
                }
                console.log("RUST LSP socket connected");
                this.lsp_client = new lspClient.LanguageClient(
                    'Custom rust LSP server',
                    async () => {
                        if (this.lsp_socket === undefined) {
                            return Promise.reject("this.lsp_socket is undefined, that should not happen");
                        }
                        return Promise.resolve({
                            reader: this.lsp_socket,
                            writer: this.lsp_socket
                        });
                    },
                    this.lsp_client_options
                );
                this.lsp_disposable = this.lsp_client.start();
                console.log(`RUST START`);
                try {
                    await this.lsp_client.onReady();
                    if (generation !== this.lifecycleGeneration) {
                        finish();
                        return;
                    }
                    this.reconnectAttempts = 0;
                    console.log(`RUST /START`);
                    await this.read_caps();
                    await this.fetch_toolbox_config().catch(error => console.log(["fetch_toolbox_config", error]));
                } catch (e) {
                    console.log(`RUST START PROBLEM e=${e}`);
                }
                finish();
            });
            socket.connect(lspPort, "127.0.0.1");
            setTimeout(() => {
                if (!settled) {
                    console.log("RUST LSP socket connect timeout");
                    socket.destroy();
                    finish();
                }
            }, 10000);
        });
    }

    async rag_status() {
        try {
            let url = this.rust_url();
            if (!url) {
                return Promise.reject("rag status no rust binary working, very strange");
            }
            url += "v1/rag-status";
            let req = new fetchH2.Request(url, {
                method: "GET",
                redirect: "follow",
                cache: "no-cache",
                referrer: "no-referrer",
            });
            let resp = await fetchH2.fetch(req, { timeout: 5000 });
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
        const rust_url = this.rust_url();

        if (!rust_url) {
            console.log(["fetch_toolbox_config: No rust binary working"]);
            return Promise.reject("No rust binary working");
        }
        const url = rust_url + "v1/customization";

        const request = new fetchH2.Request(url, { method: "GET" });

        const response = await fetchH2.fetch(request, { timeout: 5000 });

        if (!response.ok) {
            console.log([
                "fetch_toolbox_config: Error fetching toolbox config",
                response.status,
                url,
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

    private schedule_lsp_reconnect(generation: number) {
        if (this.reconnectTimer || generation !== this.lifecycleGeneration) {
            return;
        }
        const delay = Math.min(30000, 1000 * Math.pow(2, this.reconnectAttempts++));
        console.log(`RUST scheduling daemon LSP reconnect in ${delay}ms`);
        this.reconnectTimer = setTimeout(() => {
            this.reconnectTimer = undefined;
            if (generation !== this.lifecycleGeneration || this.reconnectGeneration !== generation) {
                return;
            }
            this.enqueueLifecycle(async (nextGeneration) => this.attach_daemon_serialized(nextGeneration));
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
        return Number.isFinite(port) && port !== undefined && port > 0
            ? Math.trunc(port)
            : refactDaemon.DEFAULT_DAEMON_PORT;
    }

    private plugin_version(): string | undefined {
        return vscode.extensions.getExtension("smallcloud.codify")?.packageJSON?.version;
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
            ast_max_files: config.get<number>("refactai.astFileLimit") ?? 35000,
            vecdb_max_files: config.get<number>("refactai.vecdbFileLimit") ?? 15000,
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

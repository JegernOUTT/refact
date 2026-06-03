/* eslint-disable @typescript-eslint/naming-convention */
import * as vscode from 'vscode';
import * as fetchH2 from 'fetch-h2';
import * as fetchAPI from "./fetchAPI";
import { join } from 'path';
import * as lspClient from 'vscode-languageclient/node';
import * as net from 'net';
import * as os from 'os';
import { register_commands } from './rconsoleCommands';
import { QuickActionProvider } from './quickProvider';


const DEBUG_HTTP_PORT = 8001;
const DEBUG_LSP_PORT = 8002;


export class RustBinaryBlob {
    public asset_path: string;
    public cmdline: string[] = [];
    public port: number = 0;
    public lsp_disposable: vscode.Disposable | undefined = undefined;
    public lsp_client: lspClient.LanguageClient | undefined = undefined;
    public lsp_socket: net.Socket | undefined = undefined;
    public lsp_client_options: lspClient.LanguageClientOptions;
    public ping_response: string = "";
    private lifecycleQueue: Promise<void> = Promise.resolve();
    private lifecycleGeneration: number = 0;

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
        let xdebug = this.x_debug();
        if (xdebug) {
            return 8001;
        } else {
            return this.port;
        }
    }

    public rust_url(): string {
        let xdebug = this.x_debug();
        let port = xdebug ? 8001 : this.port;
        if (!port) {
            return "";
        }
        return "http2://127.0.0.1:" + port.toString() + "/";
    }

    public browser_url(): string {
        const port = this.get_port();
        if (!port) {
            return "";
        }
        const configuredHost = vscode.workspace.getConfiguration().get<string>("refactai.browserHost")?.trim();
        const host = configuredHost && configuredHost !== "0.0.0.0" ? configuredHost : this.default_browser_host();
        return `http://${host}:${port}/`;
    }

    private default_browser_host(): string {
        return this.default_mdns_host();
    }

    private default_lan_ipv4_host(): string | undefined {
        for (const infos of Object.values(os.networkInterfaces())) {
            for (const info of infos ?? []) {
                if (
                    info.family === "IPv4" &&
                    !info.internal &&
                    !info.address.startsWith("169.254.") &&
                    info.address !== "0.0.0.0"
                ) {
                    return info.address;
                }
            }
        }
        return undefined;
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
        let xdebug = this.x_debug();
        if (xdebug) {
            return `debug rust binary on ports ${DEBUG_HTTP_PORT} and ${DEBUG_LSP_PORT}`;
        }
        return "local Refact engine";
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
            for (let i = 0; i < 5; i++) {
                if (generation !== this.lifecycleGeneration) {
                    return;
                }
                console.log(`RUST settings changed, attempt to restart ${i + 1}`);
                let xdebug = this.x_debug();
                let port: number;
                let ping_response: string;

                if (xdebug === 0) {
                    if (this.lsp_client) { // running
                        port = this.port;  // keep the same port
                        ping_response = this.ping_response;
                    } else {
                        port = Math.floor(Math.random() * 20) + 9080;
                        ping_response = `ping-${Math.floor(Math.random() * 0x10000000000000000).toString(16)}`;
                    }
                } else {
                    port = DEBUG_HTTP_PORT;
                    console.log(`RUST debug is set, don't start the rust binary. Will attempt HTTP port ${DEBUG_HTTP_PORT}, LSP port ${DEBUG_LSP_PORT}`);
                    console.log("Also, will try to read caps. If that fails, things like lists of available models will be empty.");
                    this.cmdline = [];
                    await this.terminate_serialized(generation);  // terminate our own
                    if (generation !== this.lifecycleGeneration) {
                        return;
                    }
                    await this.read_caps();  // debugging rust already running, can read here

                    await this.fetch_toolbox_config();
                    // await register_commands();
                    await this.start_lsp_socket(generation);
                    return;
                }
                const httpHost = vscode.workspace.getConfiguration().get<string>("refactai.httpHost")?.trim() || "0.0.0.0";
                let new_cmdline: string[] = [
                    join(this.asset_path, "refact-lsp"),
                    "--ping-message", ping_response,
                    "--http-port", port.toString(),
                    "--http-host", httpHost,
                    "--lsp-stdin-stdout", "1",
                ];

                if (vscode.workspace.getConfiguration().get<boolean>("refactai.vecdb")) {
                    new_cmdline.push("--vecdb");
                    const vecdb_limit = vscode.workspace.getConfiguration().get<number>("refactai.vecdbFileLimit") ?? 15000;
                    new_cmdline.push(`--vecdb-max-files`);
                    new_cmdline.push(`${vecdb_limit}`);
                }
                if (vscode.workspace.getConfiguration().get<boolean>("refactai.ast")) {
                    new_cmdline.push("--ast");
                    const ast_limit = vscode.workspace.getConfiguration().get<number>("refactai.astFileLimit") ?? 15000;
                    new_cmdline.push(`--ast-max-files`);
                    new_cmdline.push(`${ast_limit}`);
                }
                let insecureSSL = vscode.workspace.getConfiguration().get("refactai.insecureSSL");
                if (insecureSSL) {
                    new_cmdline.push("--insecure");
                }
                let experimental = vscode.workspace.getConfiguration().get("refactai.xperimental");
                if (experimental) {
                    new_cmdline.push("--experimental");
                }

                let cmdline_existing: string = this.cmdline.join(" ");
                let cmdline_new: string = new_cmdline.join(" ");
                if (cmdline_existing !== cmdline_new) {
                    this.cmdline = new_cmdline;
                    this.port = port;
                    this.ping_response = ping_response;
                    await this.launch_serialized(generation);
                }
                if (this.lsp_disposable !== undefined) {
                    break;
                }
            }
        } finally {
            global.side_panel?.handleSettingsChange();
        }
    }

    public async launch() {
        return this.enqueueLifecycle(async (generation) => this.launch_serialized(generation));
    }

    private async launch_serialized(generation: number) {
        await this.terminate_serialized(generation);
        if (generation !== this.lifecycleGeneration) {
            return;
        }
        let xdebug = this.x_debug();
        if (xdebug) {
            await this.start_lsp_socket(generation);
        } else {
            await this.start_lsp_stdin_stdout(generation);
        }
    }

    public async stop_lsp(generation: number = this.lifecycleGeneration) {
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
        this.lsp_client = undefined;
        this.lsp_socket = undefined;
    }

    public async terminate() {
        return this.enqueueLifecycle(async (generation) => this.terminate_serialized(generation));
    }

    private async terminate_serialized(generation: number) {
        await this.stop_lsp(generation);
        if (generation !== this.lifecycleGeneration) {
            return;
        }
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

        const promptCustomization = await fetchAPI.get_prompt_customization();
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
                return Promise.reject("ping bad status");
            }
            let pong = await resp.text();
            let success = (pong === this.ping_response || pong === this.ping_response + "\n");
            console.log([`pong=${pong}`, `expected ${this.ping_response}`, success]);
            return success;
        } catch (e) {
            console.log(["ping error:", e]);
        }
        return false;
    }

    public async start_lsp_stdin_stdout(generation: number = this.lifecycleGeneration) {
        console.log("RUST start_lsp_stdint_stdout");
        let path = this.cmdline[0];
        let serverOptions: lspClient.ServerOptions;
        serverOptions = {
            run: {
                command: String(path),
                args: this.cmdline.slice(1),
                transport: lspClient.TransportKind.stdio,
                options: { cwd: process.cwd(), detached: false, shell: false }
            },
            debug: {
                command: String(path),
                args: this.cmdline.slice(1),
                transport: lspClient.TransportKind.stdio,
                options: { cwd: process.cwd(), detached: false, shell: false }
            }
        };
        this.lsp_client = new lspClient.LanguageClient(
            'RUST LSP',
            serverOptions,
            this.lsp_client_options
        );
        this.lsp_disposable = this.lsp_client.start();

        console.log(`${logts()} RUST START`);
        const somethings_wrong_timeout = 10000;
        const startTime = Date.now();
        let started_okay = false;

        const onReadyPromise = this.lsp_client.onReady().then(() => {
            started_okay = true;
        });

        try {
            while (true) {
                if (generation !== this.lifecycleGeneration) {
                    return;
                }
                const elapsedTime = Date.now() - startTime;
                if (started_okay) {
                    console.log(`${logts()} RUST /START after ${elapsedTime}ms`);
                    break;
                }
                if (elapsedTime >= somethings_wrong_timeout) {
                    throw new Error("timeout");
                }
                console.log(`${logts()} RUST waiting...`);
                await new Promise(resolve => setTimeout(resolve, 100));
            }
        } catch (e) {
            console.log(`${logts()} RUST START PROBLEM e=${e}`);
            this.lsp_dispose();
            return;
        }
        if (generation !== this.lifecycleGeneration) {
            return;
        }

        let success = await this.ping();
        if (!success) {
            console.log("RUST ping failed");
            this.lsp_dispose();
            return;
        }
        if (generation !== this.lifecycleGeneration) {
            return;
        }
        // At this point we had successful client_info and workspace_folders server to client calls,
        // therefore the LSP server is started.
        // A little doubt remains about the http port, but it's very likely there's no race.
        await this.read_caps();
        await this.fetch_toolbox_config();
    }

    public async start_lsp_socket(generation: number = this.lifecycleGeneration) {
        console.log("RUST start_lsp_socket");
        this.lsp_socket = new net.Socket();
        this.lsp_socket.on('error', (error) => {
            console.log("RUST socket error");
            console.log(error);
            console.log("RUST /error");
            this.lsp_dispose();
        });
        this.lsp_socket.on('close', () => {
            console.log("RUST socket closed");
            this.lsp_dispose();
        });
        this.lsp_socket.on('connect', async () => {
            if (generation !== this.lifecycleGeneration) {
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
            // client.registerProposedFeatures();
            this.lsp_disposable = this.lsp_client.start();
            console.log(`RUST DEBUG START`);
            try {
                await this.lsp_client.onReady();
                if (generation !== this.lifecycleGeneration) {
                    return;
                }
                console.log(`RUST DEBUG /START`);
            } catch (e) {
                console.log(`RUST DEBUG START PROBLEM e=${e}`);
            }
        });
        this.lsp_socket.connect(DEBUG_LSP_PORT);
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

        // TBD: type-guards or some sort of runtime validation
        const json = await response.json() as ToolboxConfig;
        console.log(["success fetch_toolbox_config", json]);

        global.toolbox_config = json;
        await register_commands();
        return json;
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

function logts() {
    const now = new Date();
    const hours = String(now.getHours()).padStart(2, '0');
    const minutes = String(now.getMinutes()).padStart(2, '0');
    const seconds = String(now.getSeconds()).padStart(2, '0');
    const milliseconds = String(now.getMilliseconds()).padStart(3, '0');
    return `${hours}${minutes}${seconds}.${milliseconds}`;
}

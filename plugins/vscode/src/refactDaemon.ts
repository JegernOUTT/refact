/* eslint-disable @typescript-eslint/naming-convention */
import * as fetchH2 from "fetch-h2";
import { spawn } from "child_process";
import * as fs from "fs";
import * as path from "path";

export const DEFAULT_DAEMON_PORT = 8488;
export const DAEMON_POLL_TIMEOUT_MS = 15000;
export const DAEMON_SHUTDOWN_TIMEOUT_MS = 10000;
export const DAEMON_SHUTDOWN_POLL_MS = 200;

export type WorkerState =
    | "stopped"
    | "starting"
    | "ready"
    | "stopping"
    | "crashed"
    | { failed: { reason: string } };

export type WorkerInfo = {
    project_id: string;
    pid?: number | null;
    http_port: number;
    lsp_port: number;
    state: WorkerState;
    last_error?: string | null;
};

export type DaemonStatus = {
    pid: number;
    version: string;
    port: number;
    started_at_ms: number;
    uptime_secs: number;
    workers: number;
    cron_pending?: Record<string, number>;
};

export type ProjectSettings = {
    ast: boolean;
    vecdb: boolean;
    ast_max_files: number;
    vecdb_max_files: number;
};

export type OpenProjectResponse = {
    project_id: string;
    slug: string;
    root: string;
    pinned: boolean;
    worker?: WorkerInfo | null;
    cron_pending?: number | null;
};

export type DaemonClientOptions = {
    port?: number;
    timeoutMs?: number;
};

type ReadDaemonInfo = (port: number) => Promise<DaemonStatus | undefined>;
type ShutdownDaemon = (port: number, reason: string) => Promise<void>;
type IsProcessRunning = (pid: number) => boolean;

export type EnsureDaemonOptions = DaemonClientOptions & {
    pluginVersion?: string;
    spawnDaemon?: (binPath: string) => void;
    readDaemonInfo?: ReadDaemonInfo;
    shutdownDaemon?: ShutdownDaemon;
    isProcessRunning?: IsProcessRunning;
    shutdownTimeoutMs?: number;
    shutdownPollMs?: number;
    sleep?: (ms: number) => Promise<void>;
    now?: () => number;
};

export type OpenProjectOptions = DaemonClientOptions & {
    clientKind?: string;
    settings?: ProjectSettings;
};

type RequestOptions = Partial<fetchH2.RequestInit>;

function normalizeDaemonPort(port: number | undefined): number {
    return Number.isFinite(port) && port !== undefined && port > 0
        ? Math.trunc(port)
        : DEFAULT_DAEMON_PORT;
}

function daemonBaseUrl(port: number | undefined): string {
    return `http://127.0.0.1:${normalizeDaemonPort(port)}`;
}

export function daemonStatusUrl(port: number = DEFAULT_DAEMON_PORT): string {
    return `${daemonBaseUrl(port)}/daemon/v1/status`;
}

export function daemonShutdownUrl(port: number = DEFAULT_DAEMON_PORT): string {
    return `${daemonBaseUrl(port)}/daemon/v1/shutdown`;
}

export function daemonOpenProjectUrl(port: number = DEFAULT_DAEMON_PORT): string {
    return `${daemonBaseUrl(port)}/daemon/v1/projects/open`;
}

export function projectProxyBaseUrl(port: number, projectId: string): string {
    return `${daemonBaseUrl(port)}/p/${encodeURIComponent(projectId)}/`;
}

export function browserProjectUrl(host: string, port: number, projectId: string): string {
    return `http://${host}:${normalizeDaemonPort(port)}/p/${encodeURIComponent(projectId)}/`;
}

export function compareVersions(left: string | undefined, right: string | undefined): number {
    const leftParts = parseVersion(left);
    const rightParts = parseVersion(right);
    for (let i = 0; i < 3; i++) {
        const diff = leftParts[i] - rightParts[i];
        if (diff !== 0) {
            return diff > 0 ? 1 : -1;
        }
    }
    return 0;
}

export function isPluginNewerThanDaemon(pluginVersion: string | undefined, daemonVersion: string | undefined): boolean {
    if (!pluginVersion || !daemonVersion) {
        return false;
    }
    return compareVersions(pluginVersion, daemonVersion) > 0;
}

function parseVersion(version: string | undefined): [number, number, number] {
    const parts = (version ?? "")
        .trim()
        .split(/[.\-+_\s]/)
        .map(part => {
            const match = part.match(/^\d+/);
            return match ? Number.parseInt(match[0], 10) : 0;
        });
    return [parts[0] ?? 0, parts[1] ?? 0, parts[2] ?? 0];
}

export function missingBundledRefactError(assetPath: string): string {
    return `refact binary not found in ${assetPath} — reinstall the extension`;
}

function bundledRefactName(): string {
    return process.platform === "win32" ? "refact.exe" : "refact";
}

export function resolveBundledRefactPath(assetPath: string): string {
    return path.join(assetPath, bundledRefactName());
}

export function ensureBundledRefactPath(assetPath: string): string {
    const binPath = resolveBundledRefactPath(assetPath);
    if (!fs.existsSync(binPath)) {
        throw new Error(missingBundledRefactError(assetPath));
    }
    return binPath;
}

function ensureDaemonSpawnTarget(binPath: string): void {
    if (!fs.existsSync(binPath)) {
        const message = `refact binary not found at ${binPath}`;
        console.log(message);
        throw new Error(message);
    }
}

export async function readDaemonInfo(port: number = DEFAULT_DAEMON_PORT, timeoutMs = 2000): Promise<DaemonStatus | undefined> {
    try {
        return await requestJson<DaemonStatus>(daemonStatusUrl(port), { method: "GET" }, timeoutMs);
    } catch (error) {
        console.log(["readDaemonInfo", error]);
        return undefined;
    }
}

export async function ensureDaemon(binPath: string, options: EnsureDaemonOptions = {}): Promise<DaemonStatus> {
    const port = normalizeDaemonPort(options.port);
    const timeoutMs = options.timeoutMs ?? DAEMON_POLL_TIMEOUT_MS;
    const shutdownTimeoutMs = options.shutdownTimeoutMs ?? DAEMON_SHUTDOWN_TIMEOUT_MS;
    const shutdownPollMs = Math.max(1, Math.trunc(options.shutdownPollMs ?? DAEMON_SHUTDOWN_POLL_MS));
    const sleep = options.sleep ?? delay;
    const now = options.now ?? Date.now;
    const spawnDaemon = options.spawnDaemon ?? defaultSpawnDaemon;
    const readInfo = options.readDaemonInfo ?? readDaemonInfo;
    const requestShutdown = options.shutdownDaemon ?? shutdownDaemon;
    const isProcessRunning = options.isProcessRunning ?? defaultIsProcessRunning;
    ensureDaemonSpawnTarget(binPath);
    const current = await readInfo(port);

    if (current && !isPluginNewerThanDaemon(options.pluginVersion, current.version)) {
        return current;
    }

    if (current) {
        await requestShutdown(port, "upgrade").catch(error => console.log(["shutdownDaemon", error]));
        await waitForOldDaemonExit(port, current.pid, shutdownTimeoutMs, shutdownPollMs, sleep, now, readInfo, isProcessRunning);
    }

    spawnDaemon(binPath);
    const minimumVersion = current ? options.pluginVersion : undefined;
    return pollDaemon(port, timeoutMs, minimumVersion, sleep, now, readInfo);
}

export async function openProject(root: string, options: OpenProjectOptions = {}): Promise<OpenProjectResponse> {
    const payload = {
        root,
        client_kind: options.clientKind ?? "vscode",
        settings: options.settings,
    };
    return requestJson<OpenProjectResponse>(
        daemonOpenProjectUrl(options.port),
        {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(payload),
        },
        options.timeoutMs ?? 120000,
    );
}

export async function detach(): Promise<void> {
    return Promise.resolve();
}

async function shutdownDaemon(port: number, reason: string): Promise<void> {
    await requestJson<unknown>(
        daemonShutdownUrl(port),
        {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ reason }),
        },
        2000,
    );
}

async function waitForOldDaemonExit(
    port: number,
    oldPid: number,
    timeoutMs: number,
    pollMs: number,
    sleep: (ms: number) => Promise<void>,
    now: () => number,
    readInfo: ReadDaemonInfo,
    isProcessRunning: IsProcessRunning,
): Promise<void> {
    const deadline = now() + timeoutMs;
    while (true) {
        const status = await readInfo(port);
        if (!status || !isProcessRunning(oldPid)) {
            return;
        }
        const remainingMs = deadline - now();
        if (remainingMs <= 0) {
            throw new Error(daemonShutdownTimeoutMessage(port, oldPid, timeoutMs));
        }
        await sleep(Math.min(pollMs, remainingMs));
    }
}

function daemonShutdownTimeoutMessage(port: number, pid: number, timeoutMs: number): string {
    const seconds = Math.ceil(timeoutMs / 1000);
    return `Refact daemon on port ${port} did not exit within ${seconds}s after shutdown. Retry shortly, or stop process ${pid} before retrying.`;
}

async function pollDaemon(
    port: number,
    timeoutMs: number,
    minimumVersion: string | undefined,
    sleep: (ms: number) => Promise<void>,
    now: () => number,
    readInfo: ReadDaemonInfo,
): Promise<DaemonStatus> {
    const deadline = now() + timeoutMs;
    while (now() <= deadline) {
        const status = await readInfo(port);
        if (status && (!minimumVersion || compareVersions(status.version, minimumVersion) >= 0)) {
            return status;
        }
        await sleep(250);
    }
    throw new Error(`Refact daemon did not become ready on port ${port}`);
}

function defaultSpawnDaemon(binPath: string): void {
    const command = daemonSpawnCommand(binPath);
    const child = spawn(command.command, command.args, {
        detached: true,
        stdio: "ignore",
    });
    child.unref();
}

export function daemonSpawnCommand(binPath: string): { command: string; args: string[] } {
    return { command: binPath, args: ["daemon"] };
}

function defaultIsProcessRunning(pid: number): boolean {
    if (!Number.isFinite(pid) || pid <= 0) {
        return false;
    }
    try {
        process.kill(pid, 0);
        return true;
    } catch (error) {
        const code = (error as NodeJS.ErrnoException).code;
        return code === "EPERM";
    }
}

function delay(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
}

async function requestJson<T>(url: string, init: RequestOptions, timeout: number): Promise<T> {
    const request = new fetchH2.Request(url, init);
    const response = await fetchH2.fetch(request, { timeout });
    if (response.status < 200 || response.status >= 300) {
        const text = await response.text().catch(() => "");
        throw new Error(`daemon request failed ${response.status} ${url}: ${text}`);
    }
    return await response.json() as T;
}

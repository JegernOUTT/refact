/* eslint-disable @typescript-eslint/naming-convention */
import * as fetchH2 from "fetch-h2";
import { spawn } from "child_process";
import * as fs from "fs";
import * as path from "path";

export const DEFAULT_DAEMON_PORT = 8488;
export const DAEMON_POLL_TIMEOUT_MS = 15000;

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

export type EnsureDaemonOptions = DaemonClientOptions & {
    pluginVersion?: string;
    spawnDaemon?: (binPath: string) => void;
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

export function resolveBundledBinaryPath(assetPath: string): string {
    const suffix = process.platform === "win32" ? ".exe" : "";
    const preferred = path.join(assetPath, `refact${suffix}`);
    if (fs.existsSync(preferred)) {
        return preferred;
    }
    const fallback = path.join(assetPath, `refact-lsp${suffix}`);
    if (fs.existsSync(fallback)) {
        return fallback;
    }
    return preferred;
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
    const sleep = options.sleep ?? delay;
    const now = options.now ?? Date.now;
    const spawnDaemon = options.spawnDaemon ?? defaultSpawnDaemon;
    const current = await readDaemonInfo(port);

    if (current && !isPluginNewerThanDaemon(options.pluginVersion, current.version)) {
        return current;
    }

    if (current) {
        await shutdownDaemon(port, "upgrade").catch(error => console.log(["shutdownDaemon", error]));
        await sleep(500);
    }

    spawnDaemon(binPath);
    const minimumVersion = current ? options.pluginVersion : undefined;
    return pollDaemon(port, timeoutMs, minimumVersion, sleep, now);
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

async function pollDaemon(
    port: number,
    timeoutMs: number,
    minimumVersion: string | undefined,
    sleep: (ms: number) => Promise<void>,
    now: () => number,
): Promise<DaemonStatus> {
    const deadline = now() + timeoutMs;
    while (now() <= deadline) {
        const status = await readDaemonInfo(port);
        if (status && (!minimumVersion || compareVersions(status.version, minimumVersion) >= 0)) {
            return status;
        }
        await sleep(250);
    }
    throw new Error(`Refact daemon did not become ready on port ${port}`);
}

function defaultSpawnDaemon(binPath: string): void {
    const child = spawn(binPath, ["daemon"], {
        detached: true,
        stdio: "ignore",
    });
    child.unref();
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

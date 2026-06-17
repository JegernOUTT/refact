/* eslint-disable @typescript-eslint/naming-convention */
import * as fetchH2 from "fetch-h2";
import { spawn } from "child_process";
import * as fs from "fs";
import * as os from "os";
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
    authToken?: string | null;
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
    homeDir?: string;
    daemonJsonPath?: string;
};

export type DaemonEndpoint = {
    port: number;
    authToken?: string;
};

type ReadDaemonInfo = (port: number, authToken?: string) => Promise<DaemonStatus | undefined>;
type ShutdownDaemon = (port: number, reason: string, authToken?: string) => Promise<void>;
type IsProcessRunning = (pid: number) => boolean;

export type FindDaemonOptions = DaemonClientOptions & {
    pluginVersion?: string;
    readDaemonInfo?: ReadDaemonInfo;
};

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
    authToken?: string | null;
};

type RequestOptions = Partial<fetchH2.RequestInit>;

type DaemonInfoWire = {
    port?: number | string;
    auth_token?: string | null;
};

function normalizeDaemonPort(port: number | undefined): number {
    return Number.isFinite(port) && port !== undefined && port > 0
        ? Math.trunc(port)
        : DEFAULT_DAEMON_PORT;
}

function daemonBaseUrl(port: number | undefined): string {
    return `http://127.0.0.1:${normalizeDaemonPort(port)}`;
}

export function daemonJsonPath(homeDir: string = os.homedir()): string {
    return path.join(homeDir, ".cache", "refact", "daemon", "daemon.json");
}

export function daemonEndpoints(options: DaemonClientOptions = {}): DaemonEndpoint[] {
    const preferredPort = normalizeDaemonPort(options.port);
    const diskInfo = readDaemonInfoFromDisk(options);
    const diskPort = normalizeDiskDaemonPort(diskInfo?.port);
    const diskToken = normalizeAuthToken(diskInfo?.auth_token);
    const endpoints: DaemonEndpoint[] = [endpointWithToken(preferredPort, diskPort === preferredPort ? diskToken : undefined)];
    if (diskPort !== undefined && diskPort !== preferredPort) {
        endpoints.push(endpointWithToken(diskPort, diskToken));
    }
    return endpoints;
}

function endpointWithToken(port: number, authToken: string | undefined): DaemonEndpoint {
    return authToken ? { port, authToken } : { port };
}

function readDaemonInfoFromDisk(options: DaemonClientOptions): DaemonInfoWire | undefined {
    const infoPath = options.daemonJsonPath ?? daemonJsonPath(options.homeDir);
    try {
        const parsed = JSON.parse(fs.readFileSync(infoPath, "utf8")) as DaemonInfoWire;
        return typeof parsed === "object" && parsed !== null ? parsed : undefined;
    } catch {
        return undefined;
    }
}

function normalizeDiskDaemonPort(port: number | string | undefined): number | undefined {
    const value = typeof port === "string" ? Number.parseInt(port, 10) : port;
    if (!Number.isFinite(value) || value === undefined || value <= 0) {
        return undefined;
    }
    return Math.trunc(value);
}

function normalizeAuthToken(authToken: string | null | undefined): string | undefined {
    const token = authToken?.trim();
    return token ? token : undefined;
}

function authTokenForPort(port: number, options: DaemonClientOptions): string | undefined {
    return daemonEndpoints(options).find(endpoint => endpoint.port === port)?.authToken;
}

function mergeDaemonEndpoints(primary: DaemonEndpoint[], secondary: DaemonEndpoint[]): DaemonEndpoint[] {
    const merged: DaemonEndpoint[] = [];
    for (const endpoint of [...primary, ...secondary]) {
        if (!merged.some(candidate => candidate.port === endpoint.port && candidate.authToken === endpoint.authToken)) {
            merged.push(endpoint);
        }
    }
    return merged;
}

function daemonStatusForEndpoint(status: DaemonStatus, endpoint: DaemonEndpoint): DaemonStatus {
    const reportedPort = status.port;
    return {
        ...status,
        port: Number.isFinite(reportedPort) && reportedPort > 0 ? Math.trunc(reportedPort) : endpoint.port,
        authToken: endpoint.authToken ?? status.authToken ?? null,
    };
}

function requestHeaders(authToken: string | null | undefined, headers: Record<string, string> = {}): Record<string, string> {
    const token = normalizeAuthToken(authToken);
    return token ? { ...headers, Authorization: `Bearer ${token}` } : headers;
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
        const diff = leftParts.core[i] - rightParts.core[i];
        if (diff !== 0) {
            return diff > 0 ? 1 : -1;
        }
    }
    return comparePrerelease(leftParts.prerelease, rightParts.prerelease);
}

export function isPluginNewerThanDaemon(pluginVersion: string | undefined, daemonVersion: string | undefined): boolean {
    if (!pluginVersion || !daemonVersion) {
        return false;
    }
    return compareVersions(pluginVersion, daemonVersion) > 0;
}

type ParsedVersion = {
    core: [number, number, number];
    prerelease: string[];
};

function parseVersion(version: string | undefined): ParsedVersion {
    const match = (version ?? "")
        .trim()
        .match(/(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?/);
    if (!match) {
        return { core: [0, 0, 0], prerelease: [] };
    }
    return {
        core: [toVersionNumber(match[1]), toVersionNumber(match[2]), toVersionNumber(match[3])],
        prerelease: match[4]?.split(".").filter(part => part.length > 0) ?? [],
    };
}

function toVersionNumber(part: string | undefined): number {
    return part ? Number.parseInt(part, 10) : 0;
}

function comparePrerelease(left: string[], right: string[]): number {
    if (left.length === 0 && right.length === 0) {
        return 0;
    }
    if (left.length === 0) {
        return 1;
    }
    if (right.length === 0) {
        return -1;
    }
    const length = Math.max(left.length, right.length);
    for (let i = 0; i < length; i++) {
        const leftPart = left[i];
        const rightPart = right[i];
        if (leftPart === undefined) {
            return -1;
        }
        if (rightPart === undefined) {
            return 1;
        }
        const diff = comparePrereleaseIdentifier(leftPart, rightPart);
        if (diff !== 0) {
            return diff;
        }
    }
    return 0;
}

function comparePrereleaseIdentifier(left: string, right: string): number {
    const leftNumeric = /^\d+$/.test(left);
    const rightNumeric = /^\d+$/.test(right);
    if (leftNumeric && rightNumeric) {
        const diff = Number.parseInt(left, 10) - Number.parseInt(right, 10);
        return diff === 0 ? 0 : diff > 0 ? 1 : -1;
    }
    if (leftNumeric) {
        return -1;
    }
    if (rightNumeric) {
        return 1;
    }
    return left === right ? 0 : left > right ? 1 : -1;
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

export async function readDaemonInfo(
    port: number = DEFAULT_DAEMON_PORT,
    timeoutMsOrAuthToken: number | string | undefined = 2000,
    maybeAuthToken?: string,
): Promise<DaemonStatus | undefined> {
    const timeoutMs = typeof timeoutMsOrAuthToken === "number" ? timeoutMsOrAuthToken : 2000;
    const authToken = typeof timeoutMsOrAuthToken === "string" ? timeoutMsOrAuthToken : maybeAuthToken;
    try {
        const endpoint = endpointWithToken(normalizeDaemonPort(port), normalizeAuthToken(authToken));
        const status = await requestJson<DaemonStatus>(
            daemonStatusUrl(endpoint.port),
            { method: "GET", headers: requestHeaders(endpoint.authToken) },
            timeoutMs,
        );
        return daemonStatusForEndpoint(status, endpoint);
    } catch (error) {
        console.log(["readDaemonInfo", error]);
        return undefined;
    }
}

export async function findExistingDaemon(options: FindDaemonOptions = {}): Promise<DaemonStatus | undefined> {
    const readInfo = options.readDaemonInfo ?? readDaemonInfo;
    for (const endpoint of daemonEndpoints(options)) {
        const status = await readInfo(endpoint.port, endpoint.authToken);
        if (status && !isPluginNewerThanDaemon(options.pluginVersion, status.version)) {
            return daemonStatusForEndpoint(status, endpoint);
        }
    }
    return undefined;
}

export async function ensureDaemon(binPath: string, options: EnsureDaemonOptions = {}): Promise<DaemonStatus> {
    const preferredEndpoint = daemonEndpoints(options)[0];
    const timeoutMs = options.timeoutMs ?? DAEMON_POLL_TIMEOUT_MS;
    const shutdownTimeoutMs = options.shutdownTimeoutMs ?? DAEMON_SHUTDOWN_TIMEOUT_MS;
    const shutdownPollMs = Math.max(1, Math.trunc(options.shutdownPollMs ?? DAEMON_SHUTDOWN_POLL_MS));
    const sleep = options.sleep ?? delay;
    const now = options.now ?? Date.now;
    const spawnDaemon = options.spawnDaemon ?? defaultSpawnDaemon;
    const readInfo = options.readDaemonInfo ?? readDaemonInfo;
    const requestShutdown = options.shutdownDaemon ?? shutdownDaemon;
    const isProcessRunning = options.isProcessRunning ?? defaultIsProcessRunning;
    let current: DaemonStatus | undefined;
    let currentEndpoint: DaemonEndpoint | undefined;
    for (const endpoint of daemonEndpoints(options)) {
        const status = await readInfo(endpoint.port, endpoint.authToken);
        if (!status) {
            continue;
        }
        const candidate = daemonStatusForEndpoint(status, endpoint);
        if (!isPluginNewerThanDaemon(options.pluginVersion, candidate.version)) {
            return candidate;
        }
        if (!current) {
            current = candidate;
            currentEndpoint = endpointWithToken(candidate.port, normalizeAuthToken(candidate.authToken ?? endpoint.authToken));
        }
    }

    ensureDaemonSpawnTarget(binPath);

    if (current && currentEndpoint) {
        await requestShutdown(currentEndpoint.port, "upgrade", currentEndpoint.authToken).catch(error => console.log(["shutdownDaemon", error]));
        await waitForOldDaemonExit(currentEndpoint, current.pid, shutdownTimeoutMs, shutdownPollMs, sleep, now, readInfo, isProcessRunning);
    }

    spawnDaemon(binPath);
    const minimumVersion = current ? options.pluginVersion : undefined;
    return pollDaemon(
        () => mergeDaemonEndpoints(daemonEndpoints(options), [preferredEndpoint, ...(currentEndpoint ? [currentEndpoint] : [])]),
        timeoutMs,
        minimumVersion,
        sleep,
        now,
        readInfo,
    );
}

export async function openProject(root: string, options: OpenProjectOptions = {}): Promise<OpenProjectResponse> {
    const port = normalizeDaemonPort(options.port);
    const authToken = normalizeAuthToken(options.authToken ?? undefined) ?? authTokenForPort(port, options);
    const payload = {
        root,
        client_kind: options.clientKind ?? "vscode",
        settings: options.settings,
    };
    return requestJson<OpenProjectResponse>(
        daemonOpenProjectUrl(port),
        {
            method: "POST",
            headers: requestHeaders(authToken, { "Content-Type": "application/json" }),
            body: JSON.stringify(payload),
        },
        options.timeoutMs ?? 120000,
    );
}

export async function detach(): Promise<void> {
    return Promise.resolve();
}

async function shutdownDaemon(port: number, reason: string, authToken?: string): Promise<void> {
    await requestJson<unknown>(
        daemonShutdownUrl(port),
        {
            method: "POST",
            headers: requestHeaders(authToken, { "Content-Type": "application/json" }),
            body: JSON.stringify({ reason }),
        },
        2000,
    );
}

async function waitForOldDaemonExit(
    endpoint: DaemonEndpoint,
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
        await readInfo(endpoint.port, endpoint.authToken);
        if (!isProcessRunning(oldPid)) {
            return;
        }
        const remainingMs = deadline - now();
        if (remainingMs <= 0) {
            throw new Error(daemonShutdownTimeoutMessage(endpoint.port, oldPid, timeoutMs));
        }
        await sleep(Math.min(pollMs, remainingMs));
    }
}

function daemonShutdownTimeoutMessage(port: number, pid: number, timeoutMs: number): string {
    const seconds = Math.ceil(timeoutMs / 1000);
    return `Refact daemon on port ${port} did not exit within ${seconds}s after shutdown. Retry shortly, or stop process ${pid} before retrying.`;
}

async function pollDaemon(
    endpoints: () => DaemonEndpoint[],
    timeoutMs: number,
    minimumVersion: string | undefined,
    sleep: (ms: number) => Promise<void>,
    now: () => number,
    readInfo: ReadDaemonInfo,
): Promise<DaemonStatus> {
    const deadline = now() + timeoutMs;
    let lastPort = DEFAULT_DAEMON_PORT;
    while (now() <= deadline) {
        const candidates = endpoints();
        lastPort = candidates[0]?.port ?? lastPort;
        for (const endpoint of candidates) {
            const status = await readInfo(endpoint.port, endpoint.authToken);
            if (status && (!minimumVersion || compareVersions(status.version, minimumVersion) >= 0)) {
                return daemonStatusForEndpoint(status, endpoint);
            }
        }
        await sleep(250);
    }
    throw new Error(`Refact daemon did not become ready on port ${lastPort}`);
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

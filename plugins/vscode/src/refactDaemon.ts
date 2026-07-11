/* eslint-disable @typescript-eslint/naming-convention */
import * as fetchH2 from "fetch-h2";
import { spawn, type StdioOptions } from "child_process";
import * as crypto from "crypto";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";

export const DEFAULT_DAEMON_PORT = 8488;
export const DEFAULT_BROWSER_HOST = "127.0.0.1";
export const DAEMON_POLL_TIMEOUT_MS = 30000;
export const DAEMON_SHUTDOWN_TIMEOUT_MS = 15000;
export const DAEMON_SHUTDOWN_POLL_MS = 200;
export const DAEMON_OPEN_PROJECT_TIMEOUT_MS = 130000;

const DAEMON_LOG_TAIL_BYTES = 12000;

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
    executable_sha256?: string | null;
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
    codegraph: boolean;
    ast_max_files: number;
    vecdb_max_files: number;
    codegraph_max_files: number;
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
type ShutdownDaemon = (
    port: number,
    reason: string,
    authToken?: string,
    options?: DaemonClientOptions,
) => Promise<void>;
type IsProcessRunning = (pid: number) => boolean;
type SpawnDaemon = (binPath: string, options?: DaemonClientOptions) => void | Promise<void>;

export type FindDaemonOptions = DaemonClientOptions & {
    pluginVersion?: string;
    expectedExecutableSha256?: string;
    readDaemonInfo?: ReadDaemonInfo;
};

export type EnsureDaemonOptions = DaemonClientOptions & {
    pluginVersion?: string;
    spawnDaemon?: SpawnDaemon;
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

class DaemonRequestError extends Error {
    readonly status: number;

    constructor(status: number, url: string, body: string) {
        super(`daemon request failed ${status} ${url}: ${body}`);
        this.status = status;
    }
}

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

function daemonLogPath(options: DaemonClientOptions = {}): string {
    const infoPath = options.daemonJsonPath ?? daemonJsonPath(options.homeDir);
    return path.join(path.dirname(infoPath), "logs", "daemon.log");
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

function freshAuthTokenForPort(port: number, options: DaemonClientOptions): string | undefined {
    const diskInfo = readDaemonInfoFromDisk(options);
    const diskPort = normalizeDiskDaemonPort(diskInfo?.port);
    if (diskPort !== normalizeDaemonPort(port)) {
        return undefined;
    }
    return normalizeAuthToken(diskInfo?.auth_token);
}

function refreshedEndpointForPort(port: number, options: DaemonClientOptions, fallback?: string | null): DaemonEndpoint {
    return endpointWithToken(normalizeDaemonPort(port), freshAuthTokenForPort(port, options) ?? normalizeAuthToken(fallback ?? undefined));
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

export function daemonRequestHeaders(authToken: string | null | undefined, headers: Record<string, string> = {}): Record<string, string> {
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

export function configuredBrowserHost(configuredHost?: string | null): string | undefined {
    const host = configuredHost?.trim();
    return host && host !== "0.0.0.0" ? host : undefined;
}

export function browserProjectUrl(host: string, port: number, projectId: string, authToken?: string | null): string {
    const baseUrl = `http://${host}:${normalizeDaemonPort(port)}/p/${encodeURIComponent(projectId)}/`;
    const token = normalizeAuthToken(authToken);
    return token ? `${baseUrl}?daemon_token=${encodeURIComponent(token)}` : baseUrl;
}

export function browserProjectUrlForConfiguredHost(
    configuredHost: string | null | undefined,
    port: number,
    projectId: string,
    authToken?: string | null,
): string {
    return browserProjectUrl(configuredBrowserHost(configuredHost) ?? DEFAULT_BROWSER_HOST, port, projectId, authToken);
}

export type PrimaryWorkspaceRootSelection = {
    primary?: string;
    ignored: string[];
    warning?: string;
};

export function selectPrimaryWorkspaceRoot(roots: string[]): PrimaryWorkspaceRootSelection {
    const primary = roots[0];
    const ignored = roots.slice(1);
    if (!primary || ignored.length === 0) {
        return { primary, ignored };
    }
    const suffix = ignored.length === 1 ? "folder" : "folders";
    return {
        primary,
        ignored,
        warning: `Refact serves only the primary VS Code workspace folder (${primary}); ignoring ${ignored.length} additional file workspace ${suffix}.`,
    };
}

export function shouldRetryProjectProxyStatus(status: number): boolean {
    return status === 502 || status === 503 || status === 504;
}

export async function projectProxyFetchWithRetry<T>(
    fetchOnce: () => Promise<T>,
    reopenProject: () => Promise<void>,
    statusOf: (result: T) => number,
): Promise<T> {
    const first = await fetchOnce();
    if (!shouldRetryProjectProxyStatus(statusOf(first))) {
        return first;
    }
    await reopenProject();
    return fetchOnce();
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

export function daemonExecutableMatchesExpected(
    status: DaemonStatus,
    pluginVersion: string | undefined,
    expectedExecutableSha256: string | undefined,
): boolean {
    if (isPluginNewerThanDaemon(pluginVersion, status.version)) {
        return false;
    }
    if ((status.version ?? "").trim() !== (pluginVersion ?? "").trim()) {
        return true;
    }
    const expectedHash = expectedExecutableSha256?.trim();
    if (!expectedHash) {
        return true;
    }
    const actualHash = status.executable_sha256?.trim();
    if (!actualHash) {
        return true;
    }
    return actualHash === expectedHash;
}

type Sha256CacheEntry = {
    size: number;
    mtimeMs: number;
    sha256: string;
};

const sha256Cache = new Map<string, Sha256CacheEntry>();

export async function sha256OfFile(filePath: string): Promise<string | undefined> {
    const absolutePath = path.resolve(filePath);
    try {
        const stat = await fs.promises.stat(absolutePath);
        if (!stat.isFile()) {
            return undefined;
        }
        const cached = sha256Cache.get(absolutePath);
        if (cached && cached.size === stat.size && cached.mtimeMs === stat.mtimeMs) {
            return cached.sha256;
        }
        const sha256 = await sha256File(absolutePath);
        sha256Cache.set(absolutePath, { size: stat.size, mtimeMs: stat.mtimeMs, sha256 });
        return sha256;
    } catch {
        return undefined;
    }
}

type ParsedVersion = {
    core: [number, number, number];
    prerelease?: string[];
};

function parseVersion(version: string | undefined): ParsedVersion {
    const trimmed = (version ?? "").trim();
    const withoutPrefix = trimmed.startsWith("v") ? trimmed.slice(1) : trimmed;
    const match = /(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?/.exec(withoutPrefix);
    if (!match) {
        return { core: [0, 0, 0] };
    }
    return {
        core: [toVersionNumber(match[1]), toVersionNumber(match[2]), toVersionNumber(match[3])],
        prerelease: parsePrerelease(match[4]),
    };
}

function toVersionNumber(part: string | undefined): number {
    return Number.parseInt(part ?? "0", 10) || 0;
}

function parsePrerelease(prerelease: string | undefined): string[] | undefined {
    const parts = prerelease?.split(".").filter(part => part.length > 0);
    return parts && parts.length > 0 ? parts : undefined;
}

function comparePrerelease(left: string[] | undefined, right: string[] | undefined): number {
    if (!left && !right) {
        return 0;
    }
    if (!left) {
        return 1;
    }
    if (!right) {
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
    const stat = fs.statSync(binPath);
    if (!stat.isFile()) {
        const message = `refact binary is not a file at ${binPath}`;
        console.log(message);
        throw new Error(message);
    }
    if (process.platform !== "win32") {
        try {
            fs.accessSync(binPath, fs.constants.X_OK);
        } catch (error) {
            const message = `refact binary is not executable at ${binPath}: ${errorMessage(error)}`;
            console.log(message);
            throw new Error(message);
        }
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
            { method: "GET" },
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
        if (status && daemonExecutableMatchesExpected(status, options.pluginVersion, options.expectedExecutableSha256)) {
            return daemonStatusForEndpoint(status, endpoint);
        }
    }
    return undefined;
}

export async function ensureDaemon(binPath: string, options: EnsureDaemonOptions = {}): Promise<DaemonStatus> {
    const expectedExecutableSha256 = await sha256OfFile(binPath);
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
        if (daemonExecutableMatchesExpected(candidate, options.pluginVersion, expectedExecutableSha256)) {
            return candidate;
        }
        if (!current) {
            current = candidate;
            currentEndpoint = endpointWithToken(candidate.port, normalizeAuthToken(candidate.authToken ?? endpoint.authToken));
        }
    }

    ensureDaemonSpawnTarget(binPath);

    if (current && currentEndpoint) {
        const shutdownEndpoint = refreshedEndpointForPort(currentEndpoint.port, options, currentEndpoint.authToken);
        try {
            await requestShutdown(shutdownEndpoint.port, "upgrade", shutdownEndpoint.authToken, options);
        } catch (error) {
            throw new Error(`Refact daemon shutdown failed before upgrade on port ${shutdownEndpoint.port}: ${errorMessage(error)}`);
        }
        await waitForOldDaemonExit(shutdownEndpoint, current.pid, shutdownTimeoutMs, shutdownPollMs, sleep, now, readInfo, isProcessRunning);
        currentEndpoint = shutdownEndpoint;
    }

    try {
        await spawnDaemon(binPath, options);
    } catch (error) {
        throw new Error(daemonStartupFailureMessage(`Failed to start Refact daemon from ${binPath}: ${errorMessage(error)}`, options));
    }
    const minimumVersion = current ? options.pluginVersion : undefined;
    try {
        return await pollDaemon(
            () => mergeDaemonEndpoints(daemonEndpoints(options), [preferredEndpoint, ...(currentEndpoint ? [currentEndpoint] : [])]),
            timeoutMs,
            minimumVersion,
            options.pluginVersion,
            expectedExecutableSha256,
            sleep,
            now,
            readInfo,
        );
    } catch (error) {
        throw new Error(daemonStartupFailureMessage(errorMessage(error), options));
    }
}

export async function openProject(root: string, options: OpenProjectOptions = {}): Promise<OpenProjectResponse> {
    const port = normalizeDaemonPort(options.port);
    const authToken = normalizeAuthToken(options.authToken ?? undefined) ?? authTokenForPort(port, options);
    const payload = {
        root,
        client_kind: options.clientKind ?? "vscode",
        settings: options.settings,
    };
    return requestJsonWithAuthRetry<OpenProjectResponse>(
        port,
        authToken,
        options,
        endpoint => daemonOpenProjectUrl(endpoint.port),
        endpoint => ({
            method: "POST",
            headers: daemonRequestHeaders(endpoint.authToken, { "Content-Type": "application/json" }),
            body: JSON.stringify(payload),
        }),
        options.timeoutMs ?? DAEMON_OPEN_PROJECT_TIMEOUT_MS,
    );
}

export async function detach(): Promise<void> {
    return Promise.resolve();
}

async function shutdownDaemon(port: number, reason: string, authToken?: string, options: DaemonClientOptions = {}): Promise<void> {
    await requestJsonWithAuthRetry<unknown>(
        port,
        authToken,
        options,
        endpoint => daemonShutdownUrl(endpoint.port),
        endpoint => ({
            method: "POST",
            headers: daemonRequestHeaders(endpoint.authToken, { "Content-Type": "application/json" }),
            body: JSON.stringify({ reason }),
        }),
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
    pluginVersion: string | undefined,
    expectedExecutableSha256: string | undefined,
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
            if (status
                && (!minimumVersion || compareVersions(status.version, minimumVersion) >= 0)
                && daemonExecutableMatchesExpected(status, pluginVersion, expectedExecutableSha256)) {
                return daemonStatusForEndpoint(status, endpoint);
            }
        }
        await sleep(250);
    }
    throw new Error(`Refact daemon did not become ready on port ${lastPort}`);
}

function defaultSpawnDaemon(binPath: string, options: DaemonClientOptions = {}): Promise<void> {
    const command = daemonSpawnCommand(binPath);
    const spawnIo = daemonSpawnStdio(options);
    return new Promise((resolve, reject) => {
        let settled = false;
        const settle = (callback: () => void) => {
            if (!settled) {
                settled = true;
                spawnIo.close();
                callback();
            }
        };
        try {
            const child = spawn(command.command, command.args, {
                detached: true,
                stdio: spawnIo.stdio,
            });
            child.once("error", error => settle(() => reject(error)));
            child.once("spawn", () => {
                child.unref();
                settle(() => resolve());
            });
        } catch (error) {
            settle(() => reject(error));
        }
    });
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

function sha256File(filePath: string): Promise<string> {
    return new Promise((resolve, reject) => {
        const hash = crypto.createHash("sha256");
        const stream = fs.createReadStream(filePath);
        stream.on("data", chunk => hash.update(chunk));
        stream.once("error", reject);
        stream.once("end", () => resolve(hash.digest("hex")));
    });
}

async function requestJsonWithAuthRetry<T>(
    port: number,
    authToken: string | undefined,
    options: DaemonClientOptions,
    urlForEndpoint: (endpoint: DaemonEndpoint) => string,
    initForEndpoint: (endpoint: DaemonEndpoint) => RequestOptions,
    timeout: number,
): Promise<T> {
    const initial = endpointWithToken(normalizeDaemonPort(port), normalizeAuthToken(authToken));
    try {
        return await requestJson<T>(urlForEndpoint(initial), initForEndpoint(initial), timeout);
    } catch (error) {
        if (!isUnauthorizedError(error)) {
            throw error;
        }
        const refreshed = refreshedEndpointForPort(initial.port, options, initial.authToken);
        return requestJson<T>(urlForEndpoint(refreshed), initForEndpoint(refreshed), timeout);
    }
}

async function requestJson<T>(url: string, init: RequestOptions, timeout: number): Promise<T> {
    const request = new fetchH2.Request(url, init);
    const response = await fetchH2.fetch(request, { timeout });
    if (response.status < 200 || response.status >= 300) {
        const text = await response.text().catch(() => "");
        throw new DaemonRequestError(response.status, url, text);
    }
    return await response.json() as T;
}

function isUnauthorizedError(error: unknown): boolean {
    return error instanceof DaemonRequestError && error.status === 401;
}

function daemonSpawnStdio(options: DaemonClientOptions = {}): { stdio: StdioOptions; close: () => void } {
    try {
        const logPath = daemonLogPath(options);
        fs.mkdirSync(path.dirname(logPath), { recursive: true });
        const fd = fs.openSync(logPath, "a");
        return {
            stdio: ["ignore", fd, fd],
            close: () => closeFd(fd),
        };
    } catch {
        return {
            stdio: "ignore",
            close: () => undefined,
        };
    }
}

function closeFd(fd: number): void {
    try {
        fs.closeSync(fd);
    } catch {
        return;
    }
}

function daemonStartupFailureMessage(reason: string, options: DaemonClientOptions): string {
    const logPath = daemonLogPath(options);
    const logTail = readFileTail(logPath, DAEMON_LOG_TAIL_BYTES);
    if (logTail.trim().length > 0) {
        return `${reason}\nDaemon log (${logPath}):\n${logTail.trimEnd()}`;
    }
    return `${reason}\nNo daemon log output found at ${logPath}.`;
}

function readFileTail(filePath: string, maxBytes: number): string {
    let fd: number | undefined;
    try {
        fd = fs.openSync(filePath, "r");
        const stat = fs.fstatSync(fd);
        const length = Math.min(stat.size, maxBytes);
        const buffer = Buffer.alloc(length);
        fs.readSync(fd, buffer, 0, length, stat.size - length);
        return buffer.toString("utf8");
    } catch {
        return "";
    } finally {
        if (fd !== undefined) {
            closeFd(fd);
        }
    }
}

function errorMessage(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
}

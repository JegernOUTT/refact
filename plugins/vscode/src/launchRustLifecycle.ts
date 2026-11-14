import { backendReadyForStatus, WorkspaceRequiredError, type RefactConnectionIssue, type RefactBackendConnectionStatus } from "./backendStatus";
import * as refactDaemon from "./refactDaemon";

export const lspSocketConnectTimeoutMs = 10000;
export const lspClientReadyTimeoutMs = 20000;
export const lspClientStopTimeoutMs = 5000;

export function connectionIssueForAttachError(error: unknown, stage?: RefactConnectionIssue["stage"]): RefactConnectionIssue | null {
    if (error instanceof WorkspaceRequiredError) {
        return { code: "workspace_required" };
    }
    if (error instanceof refactDaemon.WorkspaceBusyError) {
        return error.issue;
    }
    return stage ? { code: "engine_start_failed", stage } : null;
}

export type LspSocketCloseAction = "ignore" | "reconnect" | "disconnect";

export function shouldRunLifecycleGeneration(generation: number, currentGeneration: number): boolean {
    return generation === currentGeneration;
}

export function lspSocketCloseAction(params: {
    generation: number;
    currentGeneration: number;
    reconnectGeneration: number | undefined;
    socketIsCurrent: boolean;
    debug: boolean;
}): LspSocketCloseAction {
    if (!params.socketIsCurrent || !shouldRunLifecycleGeneration(params.generation, params.currentGeneration)) {
        return "ignore";
    }
    if (!params.debug && params.reconnectGeneration === params.generation) {
        return "reconnect";
    }
    return "disconnect";
}

export function attachStateForDaemonOpenProject(): RefactBackendConnectionStatus {
    return "starting";
}

export function browserUrlForBackendStatus(params: {
    status: RefactBackendConnectionStatus;
    debug: boolean;
    debugHttpPort: number;
    port: number;
    projectId: string;
    configuredHost?: string;
    authToken?: string;
}): string {
    if (!backendReadyForStatus(params.status)) {
        return "";
    }
    if (params.debug) {
        const host = refactDaemon.configuredBrowserHost(params.configuredHost) ?? refactDaemon.DEFAULT_BROWSER_HOST;
        return `http://${host}:${params.debugHttpPort}/`;
    }
    if (!params.port || !params.projectId) {
        return "";
    }
    return refactDaemon.browserProjectUrlForConfiguredHost(
        params.configuredHost,
        params.port,
        params.projectId,
        params.authToken,
    );
}

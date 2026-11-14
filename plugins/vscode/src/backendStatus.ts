export type RefactBackendConnectionStatus = "connecting" | "starting" | "installing" | "ready" | "failed";

export type RefactConnectionIssue = {
    code: "workspace_required" | "workspace_busy" | "engine_start_failed";
    stage?: "engine_resolution" | "daemon_start" | "project_open" | "lsp_attach" | "health_check";
    holderPid?: number;
    holderHttpPort?: number;
};

export class WorkspaceRequiredError extends Error {
    constructor() {
        super("Open a workspace folder before starting Refact.");
    }
}

export type RefactBackendConfig = {
    backendReady: boolean;
    connectionStatus: RefactBackendConnectionStatus;
    connectionIssue: RefactConnectionIssue | null;
};

export function backendReadyForStatus(status: RefactBackendConnectionStatus): boolean {
    return status === "ready";
}

export function backendConfigForStatus(status: RefactBackendConnectionStatus, issue: RefactConnectionIssue | null = null): RefactBackendConfig {
    return {
        backendReady: backendReadyForStatus(status),
        connectionStatus: status,
        connectionIssue: status === "ready" ? null : issue,
    };
}

export function effectiveLspPortForStatus(port: number, status: RefactBackendConnectionStatus): number {
    return backendReadyForStatus(status) && Number.isFinite(port) && port > 0 ? port : 0;
}

export function shouldReadCapsForCompletion(haveCaps: boolean, status: RefactBackendConnectionStatus): boolean {
    return !haveCaps && backendReadyForStatus(status);
}

export function backendStatusLabel(status: RefactBackendConnectionStatus): string {
    switch (status) {
        case "connecting":
            return "Connecting to Refact";
        case "starting":
            return "Starting Refact engine";
        case "installing":
            return "Installing Refact engine";
        case "ready":
            return "Refact engine ready";
        case "failed":
            return "Refact engine failed";
    }
}

export function connectionIssueDescription(issue: RefactConnectionIssue): string {
    if (issue.code === "workspace_required") { return "Open a workspace folder before starting Refact."; }
    if (issue.code === "workspace_busy") { return "This workspace is already in use by another Refact engine."; }
    switch (issue.stage) {
        case "engine_resolution": return "Refact engine resolution failed.";
        case "daemon_start": return "Refact daemon startup failed.";
        case "project_open": return "Refact could not open the workspace.";
        case "lsp_attach": return "Refact language service connection failed.";
        case "health_check": return "Refact engine health check failed.";
        default: return "Refact engine startup failed.";
    }
}

export type RefactBackendConnectionStatus = "connecting" | "starting" | "installing" | "ready" | "failed";

export type RefactBackendConfig = {
    backendReady: boolean;
    connectionStatus: RefactBackendConnectionStatus;
};

export function backendReadyForStatus(status: RefactBackendConnectionStatus): boolean {
    return status === "ready";
}

export function backendConfigForStatus(status: RefactBackendConnectionStatus): RefactBackendConfig {
    return {
        backendReady: backendReadyForStatus(status),
        connectionStatus: status,
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

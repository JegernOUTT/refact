export const lspSocketConnectTimeoutMs = 10000;
export const lspClientReadyTimeoutMs = 20000;
export const lspClientStopTimeoutMs = 5000;

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

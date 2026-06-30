export type WebviewEndpointConfig = {
    browserUrl?: string;
    lspUrl?: string;
};

export function webviewEndpointConfig(backendReady: boolean, lspUrl: string | undefined, browserUrl: string | undefined): WebviewEndpointConfig {
    if (!backendReady) {
        return { lspUrl: undefined, browserUrl: undefined };
    }
    return {
        ...(lspUrl ? { lspUrl } : {}),
        ...(browserUrl ? { browserUrl } : {}),
    };
}

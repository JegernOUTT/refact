export type WebviewEndpointConfig = {
    browserUrl: string | undefined;
    lspUrl?: string;
};

export function webviewEndpointConfig(backendReady: boolean, lspUrl: string | undefined, browserUrl: string | undefined): WebviewEndpointConfig {
    return {
        browserUrl,
        ...(backendReady && lspUrl ? { lspUrl } : {}),
    };
}

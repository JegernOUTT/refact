export type EngineApiConfig = {
    host?: "web" | "ide" | "vscode" | "jetbrains";
    lspPort?: number;
    lspUrl?: string;
    browserUrl?: string;
    dev?: boolean;
    engineServed?: boolean;
};
export type QueryValue = string | number | boolean | null | undefined;
export type QueryParams = Record<string, QueryValue> | URLSearchParams;
export declare function sanitizeEngineBaseUrl(raw: string | undefined): string | null;
export declare function resolveEngineBaseUrl(config: EngineApiConfig): string;
export declare function hasUsableEngineEndpoint(config: EngineApiConfig): boolean;
export declare function normalizeEndpointPath(path: string): string;
export declare function buildApiUrl(config: EngineApiConfig, path: string, query?: QueryParams): string;
export declare function buildApiUrlFromState(state: {
    config: EngineApiConfig;
}, path: string, query?: QueryParams): string;
/** Legacy local/IDE fallback adapter; it cannot infer dev or engine-served relative mode. */
export declare function buildApiUrlFromParts(port: number, lspUrl: string | undefined, path: string, query?: QueryParams): string;
export declare function getEngineEndpointIdentity(config: EngineApiConfig): string;

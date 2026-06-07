import { postBuddyErrorRequest } from "../../services/refact/buddy";
import type { EngineApiConnection } from "../../services/refact/chatCommands";
export declare const BUDDY_FRONTEND_ERROR_NOISE_PATTERNS: RegExp[];
export declare function isBuddyFrontendErrorNoise(text: string): boolean;
export type BuddyFrontendErrorSource = "window_error" | "unhandledrejection" | "react_error_boundary" | "react_root_render" | "react_recoverable" | "artifact_iframe" | "ui_error_state" | "mermaid_render" | "possible_renderer_crash";
type BuddyCrashHotSlot = "tool" | "report" | "reasoning" | "tasks";
type BuddyCrashBreadcrumb = {
    ts: number;
    label: string;
    detail: string;
};
type BuddyCrashSession = {
    version: number;
    sessionId: string;
    status: "running" | "closed";
    startedAt: number;
    updatedAt: number;
    closedAt?: number;
    host?: string;
    page?: string;
    chatId?: string;
    isStreaming?: boolean;
    visibility?: string;
    userAgent?: string;
    heapUsed?: number;
    heapLimit?: number;
    hot?: Partial<Record<BuddyCrashHotSlot, string>>;
    breadcrumbs: BuddyCrashBreadcrumb[];
};
type BuddyCrashContext = {
    host?: string;
    page?: string;
    chatId?: string;
    isStreaming?: boolean;
};
export declare function beginBuddyCrashSession(context: BuddyCrashContext): BuddyCrashSession | null;
export declare function touchBuddyCrashSession(context: BuddyCrashContext): void;
export declare function closeBuddyCrashSession(reason?: string): void;
export declare function setBuddyCrashHotSlot(slot: BuddyCrashHotSlot, detail: string | null): void;
export declare function addBuddyCrashBreadcrumb(label: string, detail: unknown): void;
export declare function buildBuddyCrashRecoveryError(session: BuddyCrashSession): string;
export declare function redactBuddyFrontendErrorText(text: string): string;
export declare function redactBuddyFrontendErrorSource(source: string | undefined): string | undefined;
export declare function buildBuddyFrontendErrorDedupeKey(args: {
    source: BuddyFrontendErrorSource;
    sourceFile?: string;
    toolName?: string;
    chatId?: string;
}, normalized: string): string;
export declare function resetBuddyFrontendErrorReportCache(): void;
type BuddyFrontendReporterState = {
    config: EngineApiConnection & {
        apiKey: string | null;
        lspPort: number;
    };
};
type BuddyFrontendErrorDeps = {
    getState: () => BuddyFrontendReporterState;
    post: typeof postBuddyErrorRequest;
    now: () => number;
};
export declare function installBuddyErrorReporter(): () => void;
export declare function reportBuddyFrontendError(args: {
    source: BuddyFrontendErrorSource;
    error: unknown;
    sourceFile?: string;
    toolName?: string;
    chatId?: string;
}, deps?: BuddyFrontendErrorDeps): Promise<void>;
export {};

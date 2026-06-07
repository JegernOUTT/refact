import type { BrowserContextOversizeInfo } from "./browserSlice";
export declare function formatKB(bytes: number): string;
export declare function estimateSize(info: BrowserContextOversizeInfo, opts: {
    includeActions: boolean;
    includeConsole: boolean;
    includeNetwork: boolean;
    includeMutations: boolean;
    includeScreenshot: boolean;
    lastNActions: number;
    lastNConsole: number;
    lastNNetwork: number;
}): number;

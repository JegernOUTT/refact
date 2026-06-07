import type { AtCommandToken, ChipDisplayInfo } from "./types";
export declare function isCommandDisabled(token: AtCommandToken, hostDisabled: boolean): boolean;
export declare function getFilename(path: string): string;
export declare function getDomain(url: string): string;
export declare function getDisplayLabel(token: AtCommandToken, allTokens?: AtCommandToken[]): string;
export declare function tokenToChipInfo(token: AtCommandToken, hostDisabled: boolean, allTokens?: AtCommandToken[]): ChipDisplayInfo;

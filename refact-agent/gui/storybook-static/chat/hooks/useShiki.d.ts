import { type Highlighter, type BundledTheme } from "shiki";
declare const LIGHT_THEME: BundledTheme;
declare const DARK_THEME: BundledTheme;
export type ShikiHighlightResult = {
    html: string;
    language: string;
};
export declare function useShiki(): {
    highlighter: Highlighter | null;
    isLoading: boolean;
    error: Error | null;
    highlight: (code: string, language: string, isDark: boolean) => Promise<ShikiHighlightResult>;
    highlightSync: (code: string, language: string, isDark: boolean) => ShikiHighlightResult | null;
    isReady: boolean;
};
export { LIGHT_THEME, DARK_THEME };

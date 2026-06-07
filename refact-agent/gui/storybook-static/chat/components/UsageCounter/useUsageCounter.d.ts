import { Usage } from '../../services/refact';
export declare function useUsageCounter(): {
    shouldShow: boolean;
    currentThreadUsage: Usage | undefined;
    totalInputTokens: number;
    currentSessionTokens: number;
    isOverflown: boolean;
    isWarning: boolean;
    isContextFull: boolean;
    tokenPercentage: number;
    hasServerExecutedTools: boolean;
    isContextFromPreviousMessage: boolean;
};

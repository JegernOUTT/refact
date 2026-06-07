export type BackoffOptions = {
    baseDelay?: number;
    maxDelay?: number;
    multiplier?: number;
    jitter?: number;
};
export declare function calculateBackoff(retryCount: number, options?: BackoffOptions): number;

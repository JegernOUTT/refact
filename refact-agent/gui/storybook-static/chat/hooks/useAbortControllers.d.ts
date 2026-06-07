export declare const useAbortControllers: () => {
    addAbortController: (key: string, fn: (reason?: string) => void) => void;
    abort: (key: string, reason?: string) => void;
    removeController: (key: string) => void;
};

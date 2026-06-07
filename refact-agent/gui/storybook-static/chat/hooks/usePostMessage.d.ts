declare global {
    interface Window {
        postIntellijMessage?: (message: Record<string, unknown>) => void;
        acquireVsCodeApi?(): {
            postMessage: (message: Record<string, unknown>) => void;
        };
    }
}
export declare const usePostMessage: () => {
    (message: any, targetOrigin: string, transfer?: Transferable[]): void;
    (message: any, options?: WindowPostMessageOptions): void;
};

import type { AppDispatch } from "../app/store";
type LoadMoreHistoryOptions = {
    dispatchOverride?: AppDispatch;
};
export declare function useLoadMoreHistory(options?: LoadMoreHistoryOptions): {
    loadMore: () => Promise<void>;
    retry: () => void;
    isLoading: boolean;
    hasMore: boolean;
    error: string | null;
};
export {};

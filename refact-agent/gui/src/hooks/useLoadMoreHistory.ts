import { useCallback, useState, useRef } from "react";
import { useAppDispatch } from "./useAppDispatch";
import { useAppSelector } from "./useAppSelector";
import type { AppDispatch, RootState } from "../app/store";
import { trajectoriesApi } from "../services/refact/trajectories";
import { replaceSnapshotHistory } from "../features/History/historySlice";

type LoadMoreHistoryOptions = {
  dispatchOverride?: AppDispatch;
};

export function useLoadMoreHistory(options: LoadMoreHistoryOptions = {}) {
  const appDispatch = useAppDispatch();
  const dispatch = options.dispatchOverride ?? appDispatch;
  const pagination = useAppSelector((state) => state.history.pagination);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const loadingRef = useRef(false);
  const requestedBoundariesRef = useRef(new Set<string>());

  const loadMore = useCallback(async () => {
    if (loadingRef.current || !pagination.hasMore) return;
    if (!pagination.cursor) return;
    const requestedCursor = pagination.cursor;
    const requestedGeneration = pagination.generation;
    const boundaryKey = `${requestedGeneration}:${requestedCursor}`;
    if (requestedBoundariesRef.current.has(boundaryKey)) return;
    requestedBoundariesRef.current.add(boundaryKey);

    loadingRef.current = true;
    setIsLoading(true);
    setError(null);

    const request = dispatch(
      trajectoriesApi.endpoints.listTrajectoriesPaginated.initiate(
        {
          limit: 50,
          cursor: requestedCursor,
          displayableOnly: true,
        },
        { forceRefetch: true, subscribe: false },
      ),
    );

    try {
      const result = await request.unwrap();

      const latestPagination = dispatch(
        (_, getState: () => RootState) => getState().history.pagination,
      );
      if (
        latestPagination.cursor !== requestedCursor ||
        latestPagination.generation !== requestedGeneration
      ) {
        return;
      }

      dispatch(
        replaceSnapshotHistory({
          items: result.items,
          append: true,
          pagination: {
            cursor: result.next_cursor,
            hasMore: result.has_more,
            totalCount: result.total_count,
          },
        }),
      );
    } catch (err) {
      requestedBoundariesRef.current.delete(boundaryKey);
      setError(err instanceof Error ? err.message : "Failed to load more");
    } finally {
      request.unsubscribe();
      loadingRef.current = false;
      setIsLoading(false);
    }
  }, [dispatch, pagination.hasMore, pagination.cursor, pagination.generation]);

  const retry = useCallback(() => {
    setError(null);
    void loadMore();
  }, [loadMore]);

  return {
    loadMore,
    retry,
    isLoading,
    hasMore: pagination.hasMore,
    error,
  };
}

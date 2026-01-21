import { useCallback, useState, useRef } from "react";
import { useAppDispatch } from "./useAppDispatch";
import { useAppSelector } from "./useAppSelector";
import { trajectoriesApi } from "../services/refact/trajectories";
import {
  hydrateHistoryFromMeta,
  setPagination,
} from "../features/History/historySlice";

export function useLoadMoreHistory() {
  const dispatch = useAppDispatch();
  const pagination = useAppSelector((state) => state.history.pagination);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const loadingRef = useRef(false);
  const cursorRef = useRef<string | null>(null);

  const loadMore = useCallback(async () => {
    if (loadingRef.current || !pagination.hasMore) return;
    if (!pagination.cursor) return;
    if (cursorRef.current === pagination.cursor) return;

    loadingRef.current = true;
    cursorRef.current = pagination.cursor;
    setIsLoading(true);
    setError(null);

    try {
      const result = await dispatch(
        trajectoriesApi.endpoints.listTrajectoriesPaginated.initiate(
          {
            limit: 50,
            cursor: pagination.cursor,
          },
          { forceRefetch: true },
        ),
      ).unwrap();

      dispatch(hydrateHistoryFromMeta(result.items));
      dispatch(
        setPagination({
          cursor: result.next_cursor,
          hasMore: result.has_more,
        }),
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load more");
    } finally {
      loadingRef.current = false;
      setIsLoading(false);
    }
  }, [dispatch, pagination.hasMore, pagination.cursor]);

  const retry = useCallback(() => {
    setError(null);
    cursorRef.current = null;
  }, []);

  return {
    loadMore,
    retry,
    isLoading,
    hasMore: pagination.hasMore,
    error,
  };
}

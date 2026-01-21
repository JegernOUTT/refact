import React, { useState, useCallback } from "react";
import { Flex, Text, Tooltip, Spinner } from "@radix-ui/themes";
import {
  CheckCircledIcon,
  CrossCircledIcon,
  UpdateIcon,
} from "@radix-ui/react-icons";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import {
  selectIsFullyConnected,
  selectConnectionProblem,
  selectBackendStatus,
  selectCurrentChatSseStatus,
} from "../../features/Connection";
import { requestSseRefresh } from "../../features/Chat/Thread/actions";
import { selectCurrentThreadId } from "../../features/Chat/Thread/selectors";
import { trajectoriesApi } from "../../services/refact/trajectories";
import { tasksApi } from "../../services/refact/tasks";
import {
  hydrateHistoryFromMeta,
  setPagination,
} from "../../features/History/historySlice";
import styles from "./ConnectionStatus.module.css";

export const ConnectionStatusIndicator: React.FC = () => {
  const dispatch = useAppDispatch();
  const isConnected = useAppSelector(selectIsFullyConnected);
  const problem = useAppSelector(selectConnectionProblem);
  const backendStatus = useAppSelector(selectBackendStatus);
  const sseStatus = useAppSelector(selectCurrentChatSseStatus);
  const currentThreadId = useAppSelector(selectCurrentThreadId);
  const [isRefreshing, setIsRefreshing] = useState(false);

  const handleRefresh = useCallback(async () => {
    setIsRefreshing(true);
    try {
      if (currentThreadId) {
        dispatch(requestSseRefresh({ chatId: currentThreadId }));
      }
      const [trajectoriesResult] = await Promise.all([
        dispatch(
          trajectoriesApi.endpoints.listTrajectoriesPaginated.initiate(
            { limit: 50 },
            { forceRefetch: true },
          ),
        ).unwrap(),
        dispatch(
          tasksApi.endpoints.listTasks.initiate(undefined, {
            forceRefetch: true,
          }),
        ),
      ]);
      dispatch(hydrateHistoryFromMeta(trajectoriesResult.items));
      dispatch(
        setPagination({
          cursor: trajectoriesResult.next_cursor,
          hasMore: trajectoriesResult.has_more,
        }),
      );
    } finally {
      setIsRefreshing(false);
    }
  }, [dispatch, currentThreadId]);

  const isReconnecting =
    sseStatus === "connecting" || backendStatus === "unknown";

  if (isConnected) {
    return (
      <Tooltip content="Connected - Click to refresh">
        <button
          type="button"
          onClick={() => void handleRefresh()}
          disabled={isRefreshing}
          className={styles.statusButton}
        >
          <Flex align="center" gap="1" className={styles.indicator}>
            {isRefreshing ? (
              <Spinner size="1" />
            ) : (
              <CheckCircledIcon className={styles.iconConnected} />
            )}
          </Flex>
        </button>
      </Tooltip>
    );
  }

  return (
    <Flex align="center" gap="1">
      <Tooltip
        content={
          isReconnecting
            ? "Reconnecting..."
            : `${problem ?? "Disconnected"} - Click to retry`
        }
      >
        <button
          type="button"
          onClick={() => void handleRefresh()}
          disabled={isRefreshing || isReconnecting}
          className={styles.statusButton}
        >
          <Flex align="center" className={styles.indicator}>
            {isRefreshing ? (
              <Spinner size="1" />
            ) : isReconnecting ? (
              <UpdateIcon className={styles.iconReconnecting} />
            ) : (
              <CrossCircledIcon className={styles.iconDisconnected} />
            )}
          </Flex>
        </button>
      </Tooltip>
      {problem && !isReconnecting && (
        <Text size="1" color="gray" className={styles.problemText}>
          {problem}
        </Text>
      )}
    </Flex>
  );
};

export default ConnectionStatusIndicator;

import React, { useState, useCallback } from "react";
import { Flex, HoverCard, Spinner, Text } from "@radix-ui/themes";
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
    const trajQuery = dispatch(
      trajectoriesApi.endpoints.listTrajectoriesPaginated.initiate(
        { limit: 50 },
        { forceRefetch: true },
      ),
    );
    const tasksQuery = dispatch(
      tasksApi.endpoints.listTasks.initiate(undefined, {
        forceRefetch: true,
      }),
    );
    try {
      if (currentThreadId) {
        dispatch(requestSseRefresh({ chatId: currentThreadId }));
      }
      const trajectoriesResult = await trajQuery.unwrap();
      await tasksQuery.unwrap();
      dispatch(hydrateHistoryFromMeta(trajectoriesResult.items));
      dispatch(
        setPagination({
          cursor: trajectoriesResult.next_cursor,
          hasMore: trajectoriesResult.has_more,
        }),
      );
    } finally {
      trajQuery.unsubscribe();
      tasksQuery.unsubscribe();
      setIsRefreshing(false);
    }
  }, [dispatch, currentThreadId]);

  const isReconnecting =
    sseStatus === "connecting" || backendStatus === "unknown";

  const getStatusClass = () => {
    if (isRefreshing) return styles.statusRefreshing;
    if (isConnected) return styles.statusConnected;
    if (isReconnecting) return styles.statusReconnecting;
    return styles.statusDisconnected;
  };

  if (isConnected) {
    return (
      <HoverCard.Root>
        <HoverCard.Trigger>
          <button
            type="button"
            onClick={() => void handleRefresh()}
            disabled={isRefreshing}
            className={`${styles.statusButton} ${getStatusClass()}`}
          >
            <Flex align="center" gap="1" className={styles.indicator}>
              {isRefreshing ? (
                <Spinner size="1" />
              ) : (
                <CheckCircledIcon className={styles.iconConnected} />
              )}
            </Flex>
          </button>
        </HoverCard.Trigger>
        <HoverCard.Content size="1" side="bottom">
          <Text as="p" size="2">
            Connected - Click to refresh
          </Text>
        </HoverCard.Content>
      </HoverCard.Root>
    );
  }

  return (
    <HoverCard.Root>
      <HoverCard.Trigger>
        <button
          type="button"
          onClick={() => void handleRefresh()}
          disabled={isRefreshing || isReconnecting}
          className={`${styles.statusButton} ${getStatusClass()} ${
            isReconnecting ? styles.reconnectingPulse : ""
          }`}
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
      </HoverCard.Trigger>
      <HoverCard.Content size="1" side="bottom">
        <Text as="p" size="2">
          {isReconnecting
            ? "Reconnecting..."
            : `${problem ?? "Disconnected"} - Click to retry`}
        </Text>
      </HoverCard.Content>
    </HoverCard.Root>
  );
};

export default ConnectionStatusIndicator;

import React, { useState, useCallback } from "react";
import { CheckCircle, LoaderCircle, RefreshCw, XCircle } from "lucide-react";
import { Icon, Tooltip } from "../ui";
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

function useRefreshWorkspaceData() {
  const dispatch = useAppDispatch();
  const currentThreadId = useAppSelector(selectCurrentThreadId);

  return useCallback(async () => {
    const trajQuery = dispatch(
      trajectoriesApi.endpoints.listTrajectoriesPaginated.initiate(
        { limit: 50, displayableOnly: true },
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
          totalCount: trajectoriesResult.total_count,
        }),
      );
    } finally {
      trajQuery.unsubscribe();
      tasksQuery.unsubscribe();
    }
  }, [dispatch, currentThreadId]);
}

export const ConnectionStatusIndicator: React.FC = () => {
  const isConnected = useAppSelector(selectIsFullyConnected);
  const problem = useAppSelector(selectConnectionProblem);
  const backendStatus = useAppSelector(selectBackendStatus);
  const sseStatus = useAppSelector(selectCurrentChatSseStatus);
  const refreshWorkspaceData = useRefreshWorkspaceData();
  const [isRefreshing, setIsRefreshing] = useState(false);

  const handleRefresh = useCallback(async () => {
    setIsRefreshing(true);
    try {
      await refreshWorkspaceData();
    } finally {
      setIsRefreshing(false);
    }
  }, [refreshWorkspaceData]);

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
      <Tooltip>
        <Tooltip.Trigger asChild>
          <button
            type="button"
            onClick={() => void handleRefresh()}
            disabled={isRefreshing}
            className={`${styles.statusButton} ${getStatusClass()}`}
          >
            <span className={styles.indicator}>
              {isRefreshing ? (
                <Icon
                  icon={LoaderCircle}
                  size="sm"
                  className={styles.iconRefreshing}
                />
              ) : (
                <Icon
                  icon={CheckCircle}
                  size="sm"
                  className={styles.iconConnected}
                />
              )}
            </span>
          </button>
        </Tooltip.Trigger>
        <Tooltip.Content side="bottom">
          Connected - Click to refresh
        </Tooltip.Content>
      </Tooltip>
    );
  }

  return (
    <Tooltip>
      <Tooltip.Trigger asChild>
        <button
          type="button"
          onClick={() => void handleRefresh()}
          disabled={isRefreshing || isReconnecting}
          className={`${styles.statusButton} ${getStatusClass()} ${
            isReconnecting ? styles.reconnectingPulse : ""
          }`}
        >
          <span className={styles.indicator}>
            {isRefreshing ? (
              <Icon
                icon={LoaderCircle}
                size="sm"
                className={styles.iconRefreshing}
              />
            ) : isReconnecting ? (
              <Icon
                icon={RefreshCw}
                size="sm"
                className={styles.iconReconnecting}
              />
            ) : (
              <Icon
                icon={XCircle}
                size="sm"
                className={styles.iconDisconnected}
              />
            )}
          </span>
        </button>
      </Tooltip.Trigger>
      <Tooltip.Content side="bottom">
        {isReconnecting
          ? "Reconnecting..."
          : `${problem ?? "Disconnected"} - Click to retry`}
      </Tooltip.Content>
    </Tooltip>
  );
};

export default ConnectionStatusIndicator;

type MiniStatusState = "idle" | "working" | "success" | "warning" | "error";

const miniStatusIconClass: Record<MiniStatusState, string> = {
  idle: styles.iconIdle,
  working: styles.iconReconnecting,
  success: styles.iconConnected,
  warning: styles.iconRefreshing,
  error: styles.iconDisconnected,
};

interface MiniStatusIndicatorProps {
  label: string;
  status: MiniStatusState;
  tooltip: string;
  onRefresh?: () => void;
}

export const MiniStatusIndicator: React.FC<MiniStatusIndicatorProps> = ({
  label,
  status,
  tooltip,
  onRefresh,
}) => {
  const icon = status === "working" ? LoaderCircle : CheckCircle;

  return (
    <Tooltip>
      <Tooltip.Trigger asChild>
        <button
          type="button"
          className={`${styles.miniStatusButton} ${
            styles[`miniStatus-${status}`]
          }`}
          onClick={onRefresh}
          disabled={!onRefresh}
          aria-label={`${label} status: ${status}`}
        >
          <span className={styles.indicator}>
            <Icon
              icon={icon}
              size="sm"
              className={miniStatusIconClass[status]}
            />
          </span>
        </button>
      </Tooltip.Trigger>
      <Tooltip.Content side="bottom">{tooltip}</Tooltip.Content>
    </Tooltip>
  );
};

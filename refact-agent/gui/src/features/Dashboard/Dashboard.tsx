import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { HeroBlock } from "./components/HeroBlock/HeroBlock";
import { FilterBar } from "./components/FilterBar/FilterBar";
import { AttentionBand } from "./components/AttentionBand/AttentionBand";
import { ContinueSection } from "./components/ContinueSection/ContinueSection";
import { StreamSection } from "./components/Stream";
import styles from "./Dashboard.module.css";
import { ErrorState, LoadingState } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { selectBackendStatus } from "../Connection";
import {
  selectChatsSection,
  selectTasksSection,
} from "../Sidebar/sidebarSlice";
import { push } from "../Pages/pagesSlice";
import { restoreChat } from "../Chat/Thread";
import type { ChatHistoryItem } from "../History/historySlice";
import { tasksApi } from "../../services/refact/tasks";
import { selectTaskMetas, type StreamFilter } from "./streamSelectors";

const OfflineState: React.FC = () => {
  const backendStatus = useAppSelector(selectBackendStatus);
  const message =
    backendStatus === "offline"
      ? "Refact engine unavailable"
      : backendStatus === "unknown"
        ? "Connecting to Refact…"
        : "Reconnecting to Refact…";

  return (
    <LoadingState
      label={message}
      variant="full"
      className={styles.offlineState}
    />
  );
};

export const Dashboard: React.FC = () => {
  const containerRef = useRef<HTMLDivElement>(null);
  const dispatch = useAppDispatch();
  const backendStatus = useAppSelector(selectBackendStatus);
  const chatsSection = useAppSelector(selectChatsSection);
  const tasksSection = useAppSelector(selectTasksSection);
  const history = useAppSelector((state) => state.history.chats, {
    devModeChecks: { stabilityCheck: "never" },
  });
  const historyLoading = useAppSelector((state) => state.history.isLoading);
  const historyError = useAppSelector((state) => state.history.loadError);
  const tasks = useAppSelector(selectTaskMetas);

  const [filter, setFilter] = useState<StreamFilter>({
    kind: "all",
    query: "",
  });
  const [attentionOnly, setAttentionOnly] = useState(false);

  const isOffline = backendStatus !== "online";
  const chatsLoading = chatsSection.status === "loading" || historyLoading;
  const tasksLoading = tasksSection.status === "loading";
  const tasksError = tasksSection.error;
  const chatsError = historyError;

  // The old TasksSection pulled the task list itself once its sidebar section
  // was ready but no SSE snapshot had seeded the RTK Query cache.
  const shouldFetchTasks =
    !isOffline && !tasksLoading && !tasksError && tasks.length === 0;
  useEffect(() => {
    if (!shouldFetchTasks) return undefined;
    const query = dispatch(
      tasksApi.endpoints.listTasks.initiate(undefined, { forceRefetch: true }),
    );
    return () => query.unsubscribe();
  }, [dispatch, shouldFetchTasks]);

  const handleOpenChat = useCallback(
    (chatId: string) => {
      const item = history[chatId] as ChatHistoryItem | undefined;
      if (item) {
        dispatch(restoreChat(item));
      }
      dispatch(push({ name: "chat" }));
    },
    [dispatch, history],
  );

  const handleOpenTask = useCallback(
    (taskId: string) => {
      dispatch(push({ name: "task workspace", taskId }));
    },
    [dispatch],
  );

  const handleToggleAttention = useCallback(() => {
    setAttentionOnly((prev) => !prev);
  }, []);

  const streamFilter = useMemo<StreamFilter>(
    () => ({ kind: filter.kind, query: filter.query }),
    [filter.kind, filter.query],
  );

  const isLoading = chatsLoading || tasksLoading;
  const loadError = tasksError ?? chatsError;

  return (
    <div ref={containerRef} className={`${styles.dashboard} rf-enter`}>
      {isOffline ? (
        <OfflineState />
      ) : (
        <div className={`${styles.page} rf-stagger`}>
          <HeroBlock />

          <FilterBar
            filter={filter}
            attentionActive={attentionOnly}
            onFilterChange={setFilter}
            onToggleAttention={handleToggleAttention}
          />

          {attentionOnly ? (
            <AttentionBand
              onOpenChat={handleOpenChat}
              onOpenTask={handleOpenTask}
            />
          ) : null}

          {loadError ? (
            <ErrorState
              title="Failed to load your workspace"
              error={loadError}
              className={styles.stateBlock}
            />
          ) : isLoading ? (
            <LoadingState label="Loading" className={styles.stateBlock} />
          ) : (
            <>
              <ContinueSection
                onOpenChat={handleOpenChat}
                onOpenTask={handleOpenTask}
              />
              <StreamSection
                filter={streamFilter}
                onOpenChat={handleOpenChat}
                onOpenTask={handleOpenTask}
              />
            </>
          )}
        </div>
      )}
    </div>
  );
};

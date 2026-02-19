import React, { useCallback, useRef } from "react";
import { Box, Flex, Spinner } from "@radix-ui/themes";
import { ChatHistory, type ChatHistoryProps } from "../ChatHistory";
import { ScrollArea } from "../ScrollArea";
import {
  useAppSelector,
  useAppDispatch,
  useLoadMoreHistory,
} from "../../hooks";
import {
  ChatHistoryItem,
  deleteChatById,
  updateChatTitleById,
} from "../../features/History/historySlice";
import { push } from "../../features/Pages/pagesSlice";
import { restoreChat } from "../../features/Chat/Thread";
import { FeatureMenu } from "../../features/Config/FeatureMenu";

import { ErrorCallout } from "../Callout";
import { getErrorMessage, clearError } from "../../features/Errors/errorsSlice";
import classNames from "classnames";
import { selectHost } from "../../features/Config/configSlice";
import { selectBackendStatus } from "../../features/Connection";
import styles from "./Sidebar.module.css";
import {
  useListTasksQuery,
  useDeleteTaskMutation,
  useUpdateTaskMetaMutation,
} from "../../services/refact/tasks";

export type SidebarProps = {
  takingNotes: boolean;
  className?: string;
  style?: React.CSSProperties;
} & Omit<
  ChatHistoryProps,
  | "history"
  | "onDeleteHistoryItem"
  | "onCreateNewChat"
  | "onHistoryItemClick"
  | "currentChatId"
>;

export const Sidebar: React.FC<SidebarProps> = ({ takingNotes, style }) => {
  const dispatch = useAppDispatch();
  const globalError = useAppSelector(getErrorMessage);
  const currentHost = useAppSelector(selectHost);
  const backendStatus = useAppSelector(selectBackendStatus);
  const history = useAppSelector((app) => app.history.chats, {
    devModeChecks: { stabilityCheck: "never" },
  });
  const historyIsLoading = useAppSelector((app) => app.history.isLoading);
  const historyLoadError = useAppSelector((app) => app.history.loadError);
  const {
    data: tasks,
    isLoading: tasksIsLoading,
    isError: tasksIsError,
  } = useListTasksQuery(undefined, {
    refetchOnMountOrArgChange: true,
  });
  const [deleteTask] = useDeleteTaskMutation();
  const [updateTaskMeta] = useUpdateTaskMetaMutation();
  const {
    loadMore: loadMoreHistoryAsync,
    hasMore: hasMoreHistory,
    isLoading: isLoadingMoreHistory,
    error: loadMoreError,
    retry: retryLoadMore,
  } = useLoadMoreHistory();

  const hasConnectionError =
    backendStatus !== "unknown" && (Boolean(historyLoadError) || tasksIsError);

  const loadMoreHistory = useCallback(() => {
    void loadMoreHistoryAsync();
  }, [loadMoreHistoryAsync]);

  const onDeleteHistoryItem = useCallback(
    (id: string) => dispatch(deleteChatById(id)),
    [dispatch],
  );

  const onHistoryItemClick = useCallback(
    (thread: ChatHistoryItem) => {
      dispatch(restoreChat(thread));
      dispatch(push({ name: "chat" }));
    },
    [dispatch],
  );

  const handleTaskClick = useCallback(
    (taskId: string) => {
      dispatch(push({ name: "task workspace", taskId }));
    },
    [dispatch],
  );

  const handleDeleteTask = useCallback(
    (taskId: string) => {
      void deleteTask(taskId);
    },
    [deleteTask],
  );

  const handleRenameTask = useCallback(
    (taskId: string, newName: string) => {
      void updateTaskMeta({ taskId, name: newName });
    },
    [updateTaskMeta],
  );

  const onRenameChat = useCallback(
    (id: string, newTitle: string) => {
      dispatch(updateChatTitleById({ chatId: id, newTitle }));
    },
    [dispatch],
  );

  const scrollAreaRef = useRef<HTMLDivElement>(null);

  return (
    <Flex
      style={{
        ...style,
        flexDirection: "column",
        height: "100%",
        overflow: "hidden",
      }}
    >
      <FeatureMenu />
      <Flex mt="1">
        <Box position="absolute" ml="5" mt="2">
          <Spinner loading={takingNotes} title="taking notes" />
        </Box>
      </Flex>

      <Box style={{ overflow: "hidden", flex: 1 }}>
        <ScrollArea scrollbars="vertical" ref={scrollAreaRef}>
          <ChatHistory
            history={history}
            tasks={tasks}
            isLoading={historyIsLoading || tasksIsLoading}
            onHistoryItemClick={onHistoryItemClick}
            onDeleteHistoryItem={onDeleteHistoryItem}
            onRenameHistoryItem={onRenameChat}
            onTaskClick={handleTaskClick}
            onDeleteTask={handleDeleteTask}
            onRenameTask={handleRenameTask}
            onLoadMore={loadMoreHistory}
            hasMore={hasMoreHistory}
            isLoadingMore={isLoadingMoreHistory}
            loadMoreError={loadMoreError}
            onRetryLoadMore={retryLoadMore}
            hasConnectionError={hasConnectionError}
            compactView={true}
            noScroll={true}
            scrollContainerRef={scrollAreaRef}
          />
        </ScrollArea>
      </Box>

      {globalError && (
        <ErrorCallout
          mx="0"
          timeout={3000}
          onClick={() => dispatch(clearError())}
          className={classNames(styles.popup, {
            [styles.popup_ide]: currentHost !== "web",
          })}
          preventRetry
        >
          {globalError}
        </ErrorCallout>
      )}
    </Flex>
  );
};

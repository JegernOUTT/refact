import React, { useCallback } from "react";
import { Box, Flex, Spinner, Text, Card } from "@radix-ui/themes";
import { ChatHistory, type ChatHistoryProps } from "../ChatHistory";
import { useAppSelector, useAppDispatch } from "../../hooks";
import {
  ChatHistoryItem,
  deleteChatById,
} from "../../features/History/historySlice";
import { push } from "../../features/Pages/pagesSlice";
import { restoreChat } from "../../features/Chat/Thread";
import { FeatureMenu } from "../../features/Config/FeatureMenu";

import { ErrorCallout } from "../Callout";
import { getErrorMessage, clearError } from "../../features/Errors/errorsSlice";
import classNames from "classnames";
import { selectHost } from "../../features/Config/configSlice";
import styles from "./Sidebar.module.css";
import {
  useListTasksQuery,
  useDeleteTaskMutation,
} from "../../services/refact/tasks";
import {
  LayersIcon,
  CheckCircledIcon,
  CrossCircledIcon,
  DotFilledIcon,
  PauseIcon,
} from "@radix-ui/react-icons";
import { CloseButton } from "../Buttons/Buttons";

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
  const history = useAppSelector((app) => app.history, {
    devModeChecks: { stabilityCheck: "never" },
  });
  const { data: tasks = [] } = useListTasksQuery(undefined, {
    pollingInterval: 2000,
  });
  const [deleteTask] = useDeleteTaskMutation();

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

  const activeTasks = tasks.filter(
    (t) =>
      t.status === "active" || t.status === "planning" || t.status === "paused",
  );

  return (
    <Flex style={{ ...style, flexDirection: "column" }}>
      <FeatureMenu />
      <Flex mt="4">
        <Box position="absolute" ml="5" mt="2">
          <Spinner loading={takingNotes} title="taking notes" />
        </Box>
      </Flex>

      {activeTasks.length > 0 && (
        <Box p="2">
          <Text
            size="2"
            weight="medium"
            color="gray"
            mb="2"
            style={{ display: "block" }}
          >
            Tasks
          </Text>
          <Flex direction="column" gap="2">
            {activeTasks.map((task) => {
              const plannerState = task.planner_session_state;
              const isPlannerWorking =
                plannerState === "generating" ||
                plannerState === "executing_tools";
              const isPlannerPaused =
                plannerState === "paused" || plannerState === "waiting_ide";
              const isPlannerError = plannerState === "error";
              const isCompleted = task.status === "completed";
              const isFailed = task.status === "abandoned";
              const dateUpdated = new Date(task.updated_at);
              const dateTimeString = dateUpdated.toLocaleString();
              return (
                <Box
                  key={task.id}
                  style={{ position: "relative", width: "100%" }}
                >
                  <Card
                    style={{ width: "100%", marginBottom: "2px" }}
                    variant="surface"
                    className="rt-Button"
                    asChild
                    role="button"
                  >
                    <button
                      onClick={(event) => {
                        event.preventDefault();
                        event.stopPropagation();
                        handleTaskClick(task.id);
                      }}
                    >
                      <Flex gap="1" align="center">
                        {isPlannerWorking && (
                          <Spinner style={{ minWidth: 16, minHeight: 16 }} />
                        )}
                        {!isPlannerWorking && isPlannerPaused && (
                          <PauseIcon
                            style={{
                              minWidth: 16,
                              minHeight: 16,
                              color: "var(--yellow-9)",
                            }}
                          />
                        )}
                        {!isPlannerWorking &&
                          !isPlannerPaused &&
                          isPlannerError && (
                            <CrossCircledIcon
                              style={{
                                minWidth: 16,
                                minHeight: 16,
                                color: "var(--red-9)",
                              }}
                            />
                          )}
                        {!isPlannerWorking &&
                          !isPlannerPaused &&
                          !isPlannerError &&
                          isCompleted && (
                            <CheckCircledIcon
                              style={{
                                minWidth: 16,
                                minHeight: 16,
                                color: "var(--green-9)",
                              }}
                            />
                          )}
                        {!isPlannerWorking &&
                          !isPlannerPaused &&
                          !isPlannerError &&
                          isFailed && (
                            <CrossCircledIcon
                              style={{
                                minWidth: 16,
                                minHeight: 16,
                                color: "var(--red-9)",
                              }}
                            />
                          )}
                        {!isPlannerWorking &&
                          !isPlannerPaused &&
                          !isPlannerError &&
                          !isCompleted &&
                          !isFailed && (
                            <DotFilledIcon
                              style={{
                                minWidth: 16,
                                minHeight: 16,
                                color: "var(--gray-9)",
                              }}
                            />
                          )}
                        <Text
                          as="div"
                          size="2"
                          weight="bold"
                          style={{
                            textOverflow: "ellipsis",
                            overflow: "hidden",
                            whiteSpace: "nowrap",
                          }}
                        >
                          {task.name}
                        </Text>
                      </Flex>

                      <Flex justify="between" mt="8px">
                        <Flex gap="4">
                          <Text
                            size="1"
                            style={{
                              display: "flex",
                              gap: "4px",
                              alignItems: "center",
                            }}
                          >
                            <LayersIcon /> {task.cards_done}/{task.cards_total}
                          </Text>
                          {task.agents_active > 0 && (
                            <Text
                              size="1"
                              color="blue"
                              style={{
                                display: "flex",
                                gap: "4px",
                                alignItems: "center",
                              }}
                            >
                              <Spinner style={{ width: 12, height: 12 }} />{" "}
                              {task.agents_active}
                            </Text>
                          )}
                        </Flex>
                        <Text size="1" color="gray">
                          {dateTimeString}
                        </Text>
                      </Flex>
                    </button>
                  </Card>

                  <Flex
                    position="absolute"
                    top="6px"
                    right="6px"
                    gap="1"
                    justify="end"
                    align="center"
                  >
                    <CloseButton
                      size="1"
                      onClick={(event) => {
                        event.preventDefault();
                        event.stopPropagation();
                        handleDeleteTask(task.id);
                      }}
                      iconSize={10}
                      title="delete task"
                    />
                  </Flex>
                </Box>
              );
            })}
          </Flex>
        </Box>
      )}

      <Box p="2" pb="0">
        <Text
          size="2"
          weight="medium"
          color="gray"
          mb="2"
          style={{ display: "block" }}
        >
          Chats
        </Text>
      </Box>
      <ChatHistory
        history={history}
        onHistoryItemClick={onHistoryItemClick}
        onDeleteHistoryItem={onDeleteHistoryItem}
      />
      {/* TODO: duplicated */}
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

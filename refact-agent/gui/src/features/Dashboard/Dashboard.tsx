import React, { useRef } from "react";
import { Flex, Text } from "@radix-ui/themes";
import { TasksSection } from "./components/TasksSection/TasksSection";
import { ChatsSection } from "./components/ChatsSection/ChatsSection";
import { NavBar } from "./components/NavBar/NavBar";
import { ResizeDivider } from "./components/ResizeDivider/ResizeDivider";
import { useDashboardLayout } from "./hooks/useDashboardLayout";
import { useDashboardCollapseState } from "./hooks/useDashboardCollapseState";
import { useDashboardResize } from "./hooks/useDashboardResize";
import { BuddyDashboardScene } from "../Buddy/BuddyDashboardScene";
import styles from "./Dashboard.module.css";
import { ChatLoading } from "../../components/ChatContent/ChatLoading";
import { useAppSelector } from "../../hooks";
import { selectBackendStatus } from "../Connection";
import {
  selectHasActiveProject,
  selectTasksSnapshotReceived,
  selectTrajectoriesSnapshotReceived,
} from "../Chat/currentProject";

const OfflineState: React.FC = () => {
  const backendStatus = useAppSelector(selectBackendStatus);
  const message =
    backendStatus === "offline"
      ? "Refact engine unavailable"
      : backendStatus === "unknown"
        ? "Connecting to Refact…"
        : "Reconnecting to Refact…";

  return (
    <Flex
      direction="column"
      align="center"
      justify="center"
      gap="3"
      className={styles.offlineState}
    >
      <ChatLoading />
      <Text size="1" color="gray">
        {message}
      </Text>
    </Flex>
  );
};

export const Dashboard: React.FC = () => {
  const containerRef = useRef<HTMLDivElement>(null);
  const splitRef = useRef<HTMLDivElement>(null);
  const breakpoint = useDashboardLayout(containerRef);
  const backendStatus = useAppSelector(selectBackendStatus);
  const hasActiveProject = useAppSelector(selectHasActiveProject);
  const trajectoriesReady = useAppSelector(selectTrajectoriesSnapshotReceived);
  const tasksReady = useAppSelector(selectTasksSnapshotReceived);

  const { collapsed, toggle } = useDashboardCollapseState();
  const {
    ratio,
    handleDrag,
    reset: resetSplit,
  } = useDashboardResize(splitRef, "dashboard:v1:split_ratio", 0.5);

  const showResizeDivider = !collapsed.chats && !collapsed.tasks;
  const isOffline = backendStatus !== "online";
  const projectLoading = !hasActiveProject;
  const chatsLoading = projectLoading || !trajectoriesReady;
  const tasksLoading = projectLoading || !tasksReady;

  const chatsFlexStyle = collapsed.chats
    ? undefined
    : collapsed.tasks
      ? { flex: "1 1 0%" }
      : { flex: `0 1 ${ratio * 100}%` };

  return (
    <div
      ref={containerRef}
      className={styles.dashboard}
      data-breakpoint={breakpoint}
    >
      {isOffline ? (
        <OfflineState />
      ) : (
        <>
          <BuddyDashboardScene />

          <div className={styles.sectionDivider} />

          <div ref={splitRef} className={styles.splitContainer}>
            <div
              className={styles.chatsWrapper}
              style={chatsFlexStyle}
              data-collapsed={collapsed.chats || undefined}
            >
              <ChatsSection
                breakpoint={breakpoint}
                collapsed={collapsed.chats}
                projectLoading={chatsLoading}
                onToggleCollapsed={() => toggle("chats")}
              />
            </div>

            {showResizeDivider ? (
              <ResizeDivider onDrag={handleDrag} onReset={resetSplit} />
            ) : (
              <div className={styles.splitDivider} />
            )}

            <div
              className={styles.tasksWrapper}
              data-collapsed={collapsed.tasks || undefined}
            >
              <TasksSection
                breakpoint={breakpoint}
                collapsed={collapsed.tasks}
                projectLoading={tasksLoading}
                onToggleCollapsed={() => toggle("tasks")}
              />
            </div>
          </div>
        </>
      )}

      <NavBar />
    </div>
  );
};

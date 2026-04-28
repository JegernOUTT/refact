import React, { useRef } from "react";
import { Flex, Text } from "@radix-ui/themes";
import { TasksSection } from "./components/TasksSection/TasksSection";
import { ChatsSection } from "./components/ChatsSection/ChatsSection";
import { NavBar } from "./components/NavBar/NavBar";
import { ResizeDivider } from "./components/ResizeDivider/ResizeDivider";
import { useDashboardLayout } from "./hooks/useDashboardLayout";
import { useDashboardCollapseState } from "./hooks/useDashboardCollapseState";
import { useDashboardResize } from "./hooks/useDashboardResize";
import { BuddyPanel } from "../Buddy/BuddyPanel";
import styles from "./Dashboard.module.css";
import { ChatLoading } from "../../components/ChatContent/ChatLoading";
import { useAppSelector } from "../../hooks";
import { selectBackendStatus } from "../Connection";

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

  const { collapsed, toggle } = useDashboardCollapseState();
  const {
    ratio,
    handleDrag,
    reset: resetSplit,
  } = useDashboardResize(splitRef, "dashboard:v1:split_ratio", 0.5);

  const showResizeDivider = !collapsed.chats && !collapsed.tasks;
  const isOffline = backendStatus !== "online";

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
          <BuddyPanel />

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

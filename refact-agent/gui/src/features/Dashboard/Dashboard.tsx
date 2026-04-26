import React, { useRef } from "react";
import { Box, Flex, Text } from "@radix-ui/themes";
import { TasksSection } from "./components/TasksSection/TasksSection";
import { ChatsSection } from "./components/ChatsSection/ChatsSection";
import { NavBar } from "./components/NavBar/NavBar";
import { ResizeDivider } from "./components/ResizeDivider/ResizeDivider";
import { useDashboardLayout } from "./hooks/useDashboardLayout";
import { useDashboardCollapseState } from "./hooks/useDashboardCollapseState";
import { useDashboardResize } from "./hooks/useDashboardResize";
import { BuddyPanel } from "../Buddy/BuddyPanel";
import { useGetPing } from "../../hooks/useGetPing";
import styles from "./Dashboard.module.css";
import chatLoadingStyles from "../../components/ChatContent/ChatLoading.module.css";

const OfflineState: React.FC = () => {
  const ping = useGetPing();
  const isConnecting = ping.isLoading || ping.isUninitialized;

  return (
    <Flex
      direction="column"
      align="center"
      justify="center"
      gap="3"
      className={styles.offlineState}
    >
      <Box className={chatLoadingStyles.dotsContainer}>
        <Box className={chatLoadingStyles.dot} />
        <Box className={chatLoadingStyles.dot} />
        <Box className={chatLoadingStyles.dot} />
      </Box>
      <Text size="1" color="gray">
        {isConnecting ? "Connecting…" : "Server unavailable"}
      </Text>
    </Flex>
  );
};

export const Dashboard: React.FC = () => {
  const containerRef = useRef<HTMLDivElement>(null);
  const splitRef = useRef<HTMLDivElement>(null);
  const breakpoint = useDashboardLayout(containerRef);
  const ping = useGetPing();

  const { collapsed, toggle } = useDashboardCollapseState();
  const {
    ratio,
    handleDrag,
    reset: resetSplit,
  } = useDashboardResize(splitRef, "dashboard:v1:split_ratio", 0.5);

  const showResizeDivider = !collapsed.chats && !collapsed.tasks;
  const isOffline = !ping.data;

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

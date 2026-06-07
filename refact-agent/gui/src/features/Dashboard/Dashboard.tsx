import React, { useRef } from "react";
import { TasksSection } from "./components/TasksSection/TasksSection";
import { ChatsSection } from "./components/ChatsSection/ChatsSection";
import { NavBar } from "./components/NavBar/NavBar";
import { ResizeDivider } from "./components/ResizeDivider/ResizeDivider";
import { useDashboardLayout } from "./hooks/useDashboardLayout";
import { useDashboardCollapseState } from "./hooks/useDashboardCollapseState";
import { useDashboardResize } from "./hooks/useDashboardResize";
import { BuddyDashboardScene } from "../Buddy/BuddyDashboardScene";
import styles from "./Dashboard.module.css";
import { LoadingState, Surface } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import { selectBackendStatus } from "../Connection";
import {
  selectChatsSection,
  selectTasksSection,
} from "../Sidebar/sidebarSlice";

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
  const splitRef = useRef<HTMLDivElement>(null);
  const breakpoint = useDashboardLayout(containerRef);
  const backendStatus = useAppSelector(selectBackendStatus);
  const chatsSection = useAppSelector(selectChatsSection);
  const tasksSection = useAppSelector(selectTasksSection);

  const { collapsed, toggle } = useDashboardCollapseState();
  const {
    ratio,
    handleDrag,
    reset: resetSplit,
  } = useDashboardResize(splitRef, "dashboard:v1:split_ratio", 0.5);

  const showResizeDivider = !collapsed.chats && !collapsed.tasks;
  const isOffline = backendStatus !== "online";
  const chatsLoading = chatsSection.status === "loading";
  const tasksLoading = tasksSection.status === "loading";

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
            <Surface
              variant="plain"
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
            </Surface>

            {showResizeDivider ? (
              <ResizeDivider onDrag={handleDrag} onReset={resetSplit} />
            ) : (
              <div className={styles.splitDivider} />
            )}

            <Surface
              variant="plain"
              className={styles.tasksWrapper}
              data-collapsed={collapsed.tasks || undefined}
            >
              <TasksSection
                breakpoint={breakpoint}
                collapsed={collapsed.tasks}
                projectLoading={tasksLoading}
                loadError={tasksSection.error}
                onToggleCollapsed={() => toggle("tasks")}
              />
            </Surface>
          </div>
        </>
      )}

      <NavBar />
    </div>
  );
};

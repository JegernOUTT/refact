import React, { useRef } from "react";
import { Text } from "@radix-ui/themes";
import { ChevronDownIcon, ChevronUpIcon } from "@radix-ui/react-icons";
import { StatsStrip } from "./components/StatsStrip/StatsStrip";
import { OpenSection } from "./components/OpenSection/OpenSection";
import { TasksSection } from "./components/TasksSection/TasksSection";
import { ChatsSection } from "./components/ChatsSection/ChatsSection";
import { NavBar } from "./components/NavBar/NavBar";
import { ResizeDivider } from "./components/ResizeDivider/ResizeDivider";
import { CollapsePanel } from "../../components/shared/CollapsePanel";
import { useDashboardLayout } from "./hooks/useDashboardLayout";
import { useOpenTabsData } from "./hooks/useOpenTabsData";
import { useDashboardCollapseState } from "./hooks/useDashboardCollapseState";
import { useDashboardResize } from "./hooks/useDashboardResize";
import { SetupBanner } from "../Setup/SetupBanner";
import styles from "./Dashboard.module.css";

export const Dashboard: React.FC = () => {
  const containerRef = useRef<HTMLDivElement>(null);
  const splitRef = useRef<HTMLDivElement>(null);
  const breakpoint = useDashboardLayout(containerRef);
  const openTabs = useOpenTabsData();

  const { collapsed, toggle } = useDashboardCollapseState();
  const { ratio, handleDrag, reset: resetSplit } = useDashboardResize(splitRef, "dashboard:v1:split_ratio", 0.5);

  const hasOpenTabs = openTabs.length > 0;
  const showResizeDivider = !collapsed.chats && !collapsed.tasks;

  const chatsFlexStyle = collapsed.chats
    ? undefined
    : collapsed.tasks
      ? { flex: "1 1 0%" }
      : { flex: `0 1 ${ratio * 100}%` };

  return (
    <div ref={containerRef} className={styles.dashboard} data-breakpoint={breakpoint}>
      {/* Stats — collapsible, content-sized */}
      <div className={styles.statsBlock}>
        <button
          type="button"
          className={styles.statsHeader}
          onClick={() => toggle("stats")}
          aria-expanded={!collapsed.stats}
        >
          <Text size="1" weight="bold" color="gray" className={styles.statsLabel}>STATS</Text>
          {collapsed.stats
            ? <ChevronDownIcon width={12} height={12} color="var(--gray-9)" />
            : <ChevronUpIcon width={12} height={12} color="var(--gray-9)" />}
        </button>
        <CollapsePanel collapsed={collapsed.stats}>
          <StatsStrip breakpoint={breakpoint} />
        </CollapsePanel>
      </div>

      <div className={styles.sectionDivider} />

      {/* Open tabs — collapsible, content-sized */}
      {hasOpenTabs && (
        <>
          <OpenSection
            tabs={openTabs}
            breakpoint={breakpoint}
            collapsed={collapsed.open}
            onToggleCollapsed={() => toggle("open")}
          />
          <div className={styles.sectionDivider} />
        </>
      )}

      <SetupBanner />

      {/* Chats + Tasks — resizable split, takes remaining space */}
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

        {showResizeDivider && <ResizeDivider onDrag={handleDrag} onReset={resetSplit} />}

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

      <NavBar />
    </div>
  );
};

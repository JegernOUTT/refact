import React, { useCallback, useRef, useState } from "react";
import { StatsStrip } from "./components/StatsStrip/StatsStrip";
import { OpenSection } from "./components/OpenSection/OpenSection";
import { TasksSection } from "./components/TasksSection/TasksSection";
import { ChatsSection } from "./components/ChatsSection/ChatsSection";
import { NavBar } from "./components/NavBar/NavBar";
import { useDashboardLayout } from "./hooks/useDashboardLayout";
import { useOpenTabsData } from "./hooks/useOpenTabsData";
import { ModelsSection } from "./components/ModelsSection/ModelsSection";
import { SetupBanner } from "../Setup/SetupBanner";
import styles from "./Dashboard.module.css";

export const Dashboard: React.FC = () => {
  const containerRef = useRef<HTMLDivElement>(null);
  const breakpoint = useDashboardLayout(containerRef);
  const openTabs = useOpenTabsData();
  const [expanded, setExpanded] = useState(false);

  const toggleExpand = useCallback(() => {
    setExpanded((prev) => !prev);
  }, []);

  const hasOpenTabs = openTabs.length > 0;

  return (
    <div
      ref={containerRef}
      className={styles.dashboard}
      data-breakpoint={breakpoint}
    >
      <StatsStrip breakpoint={breakpoint} compact={expanded} />

      {!expanded && <ModelsSection breakpoint={breakpoint} />}

      <div className={styles.sectionDivider} />

      {/* Open tabs section — compact when Chats is expanded */}
      {hasOpenTabs && !expanded && (
        <OpenSection
          tabs={openTabs}
          breakpoint={breakpoint}
          compact={false}
        />
      )}

      <SetupBanner />

      {expanded && hasOpenTabs && (
        <OpenSection tabs={openTabs} breakpoint={breakpoint} compact />
      )}

      <div className={styles.sectionDivider} />

      {/* Chat history — expandable, virtualized */}
      <ChatsSection
        breakpoint={breakpoint}
        expanded={expanded}
        onToggleExpand={toggleExpand}
      />

      <div className={styles.sectionDivider} />

      {/* Tasks section — scrollable list */}
      <TasksSection breakpoint={breakpoint} />

      {/* Bottom nav bar */}
      <NavBar />
    </div>
  );
};

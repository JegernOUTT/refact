import React, { useCallback, useRef, useState } from "react";
import { StatsStrip } from "./components/StatsStrip/StatsStrip";
import { OpenSection } from "./components/OpenSection/OpenSection";
import { TasksSection } from "./components/TasksSection/TasksSection";
import { RecentSection } from "./components/RecentSection/RecentSection";
import { NavBar } from "./components/NavBar/NavBar";
import { useDashboardLayout } from "./hooks/useDashboardLayout";
import { useOpenTabsData } from "./hooks/useOpenTabsData";
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
      {/* Stats Strip — compact when RECENT is expanded */}
      <StatsStrip breakpoint={breakpoint} compact={expanded} />

      <div className={styles.sectionDivider} />

      {/* Open tabs section — compact when RECENT is expanded */}
      {hasOpenTabs && !expanded && (
        <OpenSection
          tabs={openTabs}
          breakpoint={breakpoint}
          compact={false}
        />
      )}

      <SetupBanner />

      {/* Active tasks — compact when RECENT is expanded */}
      {!expanded && <TasksSection breakpoint={breakpoint} />}
      {expanded && hasOpenTabs && (
        <OpenSection tabs={openTabs} breakpoint={breakpoint} compact />
      )}

      <div className={styles.sectionDivider} />

      {/* Recent history — expandable, virtualized */}
      <RecentSection
        breakpoint={breakpoint}
        expanded={expanded}
        onToggleExpand={toggleExpand}
      />

      {/* Bottom nav bar */}
      <NavBar />
    </div>
  );
};

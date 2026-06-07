import React, { useState, useCallback } from "react";
import { ArrowLeft } from "lucide-react";
import { Button, Tabs, SegmentedControl } from "../../components/ui";
import { PageWrapper } from "../../components/PageWrapper";
import type { Config } from "../Config/configSlice";
import type { DateRange, DateRangePreset } from "./types";
import { OverviewTab } from "./tabs/OverviewTab";
import { UsageTab } from "./tabs/UsageTab";
import { ThreadsTab } from "./tabs/ThreadsTab";
import { TasksTab } from "./tabs/TasksTab";
import styles from "./StatsDashboard.module.css";

export type StatsDashboardProps = {
  host: Config["host"];
  tabbed: Config["tabbed"];
  backFromDashboard: () => void;
};

const rangeOptions = [
  { value: "7d", label: "7 days" },
  { value: "30d", label: "30 days" },
  { value: "all", label: "All time" },
];

export const StatsDashboard: React.FC<StatsDashboardProps> = ({
  host,
  backFromDashboard,
}) => {
  const [dateRange, setDateRange] = useState<DateRange>({ preset: "7d" });
  const [activeTab, setActiveTab] = useState("overview");

  const handlePresetChange = useCallback((preset: string) => {
    setDateRange({ preset: preset as DateRangePreset });
  }, []);

  return (
    <PageWrapper host={host}>
      <div className={styles.root}>
        <header className={styles.header}>
          <Button
            className={styles.backButton}
            leftIcon={ArrowLeft}
            onClick={backFromDashboard}
            size="sm"
            variant="ghost"
          >
            Back
          </Button>
          <h2 className={styles.title}>Usage Dashboard</h2>
          <SegmentedControl
            aria-label="Usage date range"
            className={styles.rangeControls}
            onValueChange={handlePresetChange}
            options={rangeOptions}
            size="sm"
            value={dateRange.preset}
          />
        </header>

        <Tabs
          value={activeTab}
          onValueChange={setActiveTab}
          className={styles.tabsRoot}
        >
          <Tabs.List
            activeIndex={["overview", "usage", "threads", "tasks"].indexOf(
              activeTab,
            )}
            className={styles.tabsList}
          >
            <Tabs.Trigger value="overview">Overview</Tabs.Trigger>
            <Tabs.Trigger value="usage">LLM Usage</Tabs.Trigger>
            <Tabs.Trigger value="threads">Threads</Tabs.Trigger>
            <Tabs.Trigger value="tasks">Tasks &amp; Agents</Tabs.Trigger>
          </Tabs.List>

          <Tabs.Content value="overview" className={styles.tabContent}>
            <OverviewTab dateRange={dateRange} />
          </Tabs.Content>

          <Tabs.Content value="usage" className={styles.tabContent}>
            <UsageTab dateRange={dateRange} />
          </Tabs.Content>

          <Tabs.Content value="threads" className={styles.tabContent}>
            <ThreadsTab dateRange={dateRange} />
          </Tabs.Content>

          <Tabs.Content value="tasks" className={styles.tabContent}>
            <TasksTab dateRange={dateRange} />
          </Tabs.Content>
        </Tabs>
      </div>
    </PageWrapper>
  );
};

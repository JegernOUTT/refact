import React, { useState, useCallback } from "react";
import { ArrowLeft } from "lucide-react";
import { Button, Tabs, SegmentedControl } from "../../components/ui";
import { PageWrapper } from "../../components/PageWrapper";
import type { Config } from "../Config/configSlice";
import type { DateRange, DateRangePreset } from "./types";
import { daysAgoIsoDate, todayIsoDate } from "./utils/dateRange";
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
  { value: "today", label: "Today" },
  { value: "7d", label: "7 days" },
  { value: "30d", label: "30 days" },
  { value: "90d", label: "90 days" },
  { value: "all", label: "All time" },
  { value: "custom", label: "Custom" },
];

const TAB_ORDER = ["overview", "usage", "threads", "tasks"];

export const StatsDashboard: React.FC<StatsDashboardProps> = ({
  host,
  backFromDashboard,
}) => {
  const [dateRange, setDateRange] = useState<DateRange>({ preset: "7d" });
  const [activeTab, setActiveTab] = useState("overview");

  const handlePresetChange = useCallback((preset: string) => {
    const next = preset as DateRangePreset;
    if (next === "custom") {
      setDateRange((prev) => ({
        preset: "custom",
        from: prev.from ?? daysAgoIsoDate(30),
        to: prev.to ?? todayIsoDate(),
      }));
      return;
    }
    setDateRange({ preset: next });
  }, []);

  const handleFromChange = useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      const value = event.target.value || undefined;
      setDateRange((prev) => ({ ...prev, preset: "custom", from: value }));
    },
    [],
  );

  const handleToChange = useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      const value = event.target.value || undefined;
      setDateRange((prev) => ({ ...prev, preset: "custom", to: value }));
    },
    [],
  );

  const today = todayIsoDate();

  return (
    <PageWrapper host={host}>
      <div className={styles.root}>
        <header className={styles.header}>
          <Button
            leftIcon={ArrowLeft}
            onClick={backFromDashboard}
            size="sm"
            variant="ghost"
          >
            Back
          </Button>
          <h2 className={styles.title}>Usage Dashboard</h2>
          <div className={styles.rangeControls}>
            <SegmentedControl
              aria-label="Usage date range"
              onValueChange={handlePresetChange}
              options={rangeOptions}
              size="sm"
              value={dateRange.preset}
            />
            {dateRange.preset === "custom" && (
              <div className={styles.customRange}>
                <input
                  aria-label="From date"
                  className={styles.dateInput}
                  type="date"
                  max={dateRange.to ?? today}
                  value={dateRange.from ?? ""}
                  onChange={handleFromChange}
                />
                <span className={styles.customRangeSep}>→</span>
                <input
                  aria-label="To date"
                  className={styles.dateInput}
                  type="date"
                  min={dateRange.from ?? undefined}
                  max={today}
                  value={dateRange.to ?? ""}
                  onChange={handleToChange}
                />
              </div>
            )}
          </div>
        </header>

        <Tabs
          value={activeTab}
          onValueChange={setActiveTab}
          className={styles.tabsRoot}
        >
          <Tabs.List
            activeIndex={TAB_ORDER.indexOf(activeTab)}
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

          <Tabs.Content
            value="usage"
            className={`${styles.tabContent} rf-enter`}
          >
            <UsageTab dateRange={dateRange} />
          </Tabs.Content>

          <Tabs.Content
            value="threads"
            className={`${styles.tabContent} rf-enter`}
          >
            <ThreadsTab dateRange={dateRange} />
          </Tabs.Content>

          <Tabs.Content
            value="tasks"
            className={`${styles.tabContent} rf-enter`}
          >
            <TasksTab dateRange={dateRange} />
          </Tabs.Content>
        </Tabs>
      </div>
    </PageWrapper>
  );
};

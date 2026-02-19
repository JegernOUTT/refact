import React, { useState, useCallback } from "react";
import { Button, Flex, Tabs, Text, SegmentedControl } from "@radix-ui/themes";
import { ArrowLeftIcon } from "@radix-ui/react-icons";
import { PageWrapper } from "../../components/PageWrapper";
import type { Config } from "../Config/configSlice";
import type { DateRange, DateRangePreset } from "./types";
import { OverviewTab } from "./tabs/OverviewTab";
import { UsageTab } from "./tabs/UsageTab";
import { ThreadsTab } from "./tabs/ThreadsTab";
import { TasksTab } from "./tabs/TasksTab";
import { ImpactTab } from "./tabs/ImpactTab";
import styles from "./StatsDashboard.module.css";

export type StatsDashboardProps = {
  host: Config["host"];
  tabbed: Config["tabbed"];
  backFromDashboard: () => void;
};

export const StatsDashboard: React.FC<StatsDashboardProps> = ({
  host,
  tabbed,
  backFromDashboard,
}) => {
  const [dateRange, setDateRange] = useState<DateRange>({ preset: "7d" });

  const handlePresetChange = useCallback((preset: string) => {
    setDateRange({ preset: preset as DateRangePreset });
  }, []);

  return (
    <PageWrapper host={host}>
      <Flex direction="column" gap="3" style={{ height: "100%" }}>
        <Flex justify="between" align="center">
          {host === "vscode" && !tabbed ? (
            <Button variant="surface" onClick={backFromDashboard}>
              <ArrowLeftIcon width="16" height="16" />
              Back
            </Button>
          ) : (
            <Button variant="outline" onClick={backFromDashboard}>
              Back
            </Button>
          )}
          <Text size="5" weight="bold">
            Usage Dashboard
          </Text>
          <SegmentedControl.Root
            value={dateRange.preset}
            onValueChange={handlePresetChange}
            size="1"
          >
            <SegmentedControl.Item value="7d">7 days</SegmentedControl.Item>
            <SegmentedControl.Item value="30d">30 days</SegmentedControl.Item>
            <SegmentedControl.Item value="all">All time</SegmentedControl.Item>
          </SegmentedControl.Root>
        </Flex>

        <Tabs.Root defaultValue="overview" className={styles.tabsRoot}>
          <Tabs.List>
            <Tabs.Trigger value="overview">Overview</Tabs.Trigger>
            <Tabs.Trigger value="usage">LLM Usage</Tabs.Trigger>
            <Tabs.Trigger value="threads">Threads</Tabs.Trigger>
            <Tabs.Trigger value="tasks">Tasks &amp; Agents</Tabs.Trigger>
            <Tabs.Trigger value="impact">Impact</Tabs.Trigger>
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

          <Tabs.Content value="impact" className={styles.tabContent}>
            <ImpactTab />
          </Tabs.Content>
        </Tabs.Root>
      </Flex>
    </PageWrapper>
  );
};

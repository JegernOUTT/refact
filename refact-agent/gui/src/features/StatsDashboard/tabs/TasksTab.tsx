import React from "react";
import { Box, Flex, Text } from "@radix-ui/themes";
import { useGetStatsSummaryQuery } from "../../../services/refact/stats";
import { Spinner } from "../../../components/Spinner";
import { ErrorCallout } from "../../../components/Callout";
import { formatTokenCount, formatCost } from "../utils/formatters";
import { dateRangeToApiArgs } from "../utils/dateRange";
import type { DateRange } from "../types";
import styles from "./TasksTab.module.css";

type Props = { dateRange: DateRange };

export const TasksTab: React.FC<Props> = ({ dateRange }) => {
  const { data, isLoading, isError } = useGetStatsSummaryQuery(
    dateRangeToApiArgs(dateRange),
  );

  if (isLoading) return <Spinner spinning />;
  if (isError) return <ErrorCallout>Failed to load stats</ErrorCallout>;

  const taskModes = data?.by_mode.filter(
    (m) => m.mode === "task_planner" || m.mode === "task_agent",
  );

  if (!data || !taskModes || taskModes.length === 0) {
    return (
      <Text className={styles.emptyText}>No task or agent usage data yet.</Text>
    );
  }

  return (
    <Flex direction="column" gap="3">
      <Box className={styles.tableWrapper}>
        <table className={styles.table}>
          <thead>
            <tr>
              <th className={styles.th}>Mode</th>
              <th className={styles.th}>Calls</th>
              <th className={styles.th}>Total Tokens</th>
              <th className={styles.th}>Cost</th>
            </tr>
          </thead>
          <tbody>
            {taskModes.map((m) => (
              <tr key={m.mode}>
                <td className={styles.td}>{m.mode}</td>
                <td className={styles.td}>{m.total_calls}</td>
                <td className={styles.td}>
                  {formatTokenCount(m.total_tokens)}
                </td>
                <td className={styles.td}>{formatCost(m.total_cost_usd)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </Box>
    </Flex>
  );
};

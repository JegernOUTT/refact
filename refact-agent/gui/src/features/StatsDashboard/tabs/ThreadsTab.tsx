import React, { useState, useMemo } from "react";
import { Box, Flex, Text } from "@radix-ui/themes";
import { useGetStatsSummaryQuery } from "../../../services/refact/stats";
import { Spinner } from "../../../components/Spinner";
import { ErrorCallout } from "../../../components/Callout";
import { formatTokenCount, formatCost, formatDate } from "../utils/formatters";
import type { DateRange, ConversationStats } from "../types";
import styles from "./ThreadsTab.module.css";

type Props = { dateRange: DateRange };

function dateRangeArgs(dateRange: DateRange): { from?: string; to?: string } {
  if (dateRange.preset === "all") return {};
  const days = dateRange.preset === "7d" ? 7 : 30;
  const from = new Date(Date.now() - days * 24 * 60 * 60 * 1000)
    .toISOString()
    .slice(0, 10);
  return { from };
}

type SortKey = keyof Pick<
  ConversationStats,
  "total_tokens" | "total_calls" | "total_cost_usd" | "created_at"
>;

export const ThreadsTab: React.FC<Props> = ({ dateRange }) => {
  const { data, isLoading, isError } = useGetStatsSummaryQuery(
    dateRangeArgs(dateRange),
  );
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<{ key: SortKey; asc: boolean }>({
    key: "total_tokens",
    asc: false,
  });

  const conversations = useMemo(() => {
    if (!data) return [];
    let rows = [...data.top_conversations];
    if (search.trim()) {
      const q = search.toLowerCase();
      rows = rows.filter(
        (r) =>
          r.title.toLowerCase().includes(q) ||
          r.model.toLowerCase().includes(q) ||
          r.mode.toLowerCase().includes(q),
      );
    }
    rows.sort((a, b) => {
      const av = sort.key === "created_at" ? a.created_at : (a[sort.key] ?? 0);
      const bv = sort.key === "created_at" ? b.created_at : (b[sort.key] ?? 0);
      if (av < bv) return sort.asc ? -1 : 1;
      if (av > bv) return sort.asc ? 1 : -1;
      return 0;
    });
    return rows;
  }, [data, search, sort]);

  if (isLoading) return <Spinner spinning />;
  if (isError) return <ErrorCallout>Failed to load stats</ErrorCallout>;

  if (!data || data.totals.total_calls === 0) {
    return (
      <Text className={styles.emptyText}>
        No usage data yet. Start chatting to see stats!
      </Text>
    );
  }

  function toggleSort(key: SortKey) {
    setSort((prev) =>
      prev.key === key ? { key, asc: !prev.asc } : { key, asc: false },
    );
  }

  function indicator(key: SortKey) {
    if (sort.key !== key) return "";
    return sort.asc ? " ↑" : " ↓";
  }

  return (
    <Flex direction="column" gap="3">
      <input
        className={styles.searchInput}
        placeholder="Search by title, model, mode…"
        value={search}
        onChange={(e) => setSearch(e.target.value)}
      />

      {conversations.length === 0 ? (
        <Text className={styles.emptyText}>No matching conversations.</Text>
      ) : (
        <Box className={styles.tableWrapper}>
          <table className={styles.table}>
            <thead>
              <tr>
                <th
                  className={styles.th}
                  onClick={() => toggleSort("created_at")}
                >
                  Date{indicator("created_at")}
                </th>
                <th className={styles.th}>Title</th>
                <th className={styles.th}>Model</th>
                <th
                  className={styles.th}
                  onClick={() => toggleSort("total_calls")}
                >
                  Messages{indicator("total_calls")}
                </th>
                <th
                  className={styles.th}
                  onClick={() => toggleSort("total_tokens")}
                >
                  Total Tokens{indicator("total_tokens")}
                </th>
                <th
                  className={styles.th}
                  onClick={() => toggleSort("total_cost_usd")}
                >
                  Cost{indicator("total_cost_usd")}
                </th>
              </tr>
            </thead>
            <tbody>
              {conversations.map((c) => (
                <tr key={c.chat_id}>
                  <td className={styles.td}>{formatDate(c.created_at)}</td>
                  <td className={`${styles.td} ${styles.titleCell}`}>
                    {c.title || c.chat_id}
                  </td>
                  <td className={styles.td}>{c.model}</td>
                  <td className={styles.td}>{c.total_calls}</td>
                  <td className={styles.td}>{formatTokenCount(c.total_tokens)}</td>
                  <td className={styles.td}>{formatCost(c.total_cost_usd)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Box>
      )}
    </Flex>
  );
};

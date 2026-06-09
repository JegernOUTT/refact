import React, { useState, useMemo } from "react";
import { Surface } from "../../../components/ui";
import { useListTrajectoriesPaginatedQuery } from "../../../services/refact/trajectories";
import { Spinner } from "../../../components/Spinner";
import { ErrorCallout } from "../../../components/Callout";
import {
  formatTokenCount,
  formatCostDisplay,
  formatDate,
} from "../utils/formatters";
import { dateRangeToApiArgs } from "../utils/dateRange";
import type { DateRange } from "../types";
import styles from "./ThreadsTab.module.css";

type Props = { dateRange: DateRange };

type SortKey =
  | "total_tokens"
  | "message_count"
  | "total_cost_usd"
  | "updated_at";

export const ThreadsTab: React.FC<Props> = ({ dateRange }) => {
  const {
    data: trajData,
    isLoading,
    isError,
  } = useListTrajectoriesPaginatedQuery({ limit: 200 });
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<{ key: SortKey; asc: boolean }>({
    key: "total_tokens",
    asc: false,
  });

  const dateArgs = dateRangeToApiArgs(dateRange);

  const items = useMemo(() => {
    if (!trajData) return [];
    let rows = trajData.items.filter((item) => {
      if (dateArgs.from) {
        if (item.updated_at < dateArgs.from) return false;
      }
      if (dateArgs.to) {
        if (item.updated_at > dateArgs.to) return false;
      }
      return true;
    });
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
      let av: string | number;
      let bv: string | number;
      if (sort.key === "updated_at") {
        av = a.updated_at;
        bv = b.updated_at;
      } else if (sort.key === "message_count") {
        av = a.message_count;
        bv = b.message_count;
      } else {
        av = a[sort.key] ?? 0;
        bv = b[sort.key] ?? 0;
      }
      if (av < bv) return sort.asc ? -1 : 1;
      if (av > bv) return sort.asc ? 1 : -1;
      return 0;
    });
    return rows;
  }, [trajData, search, sort, dateArgs]);

  if (isLoading) return <Spinner spinning />;
  if (isError) return <ErrorCallout>Failed to load threads</ErrorCallout>;

  if (!trajData || items.length === 0) {
    return (
      <p className={styles.emptyText}>
        No threads yet. Start chatting to see stats!
      </p>
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
    <div className={styles.root}>
      <input
        className={styles.searchInput}
        placeholder="Search by title, model, mode…"
        value={search}
        onChange={(e) => setSearch(e.target.value)}
      />

      {items.length === 0 ? (
        <p className={styles.emptyText}>No matching threads.</p>
      ) : (
        <Surface
          animated="rise"
          className={styles.tableWrapper}
          variant="glass"
        >
          <table className={styles.table}>
            <thead>
              <tr>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={`${styles.sortButton} rf-pressable`}
                    onClick={() => toggleSort("updated_at")}
                  >
                    Date{indicator("updated_at")}
                  </button>
                </th>
                <th className={styles.th}>Title</th>
                <th className={styles.th}>Model</th>
                <th className={styles.th}>Mode</th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={`${styles.sortButton} rf-pressable`}
                    onClick={() => toggleSort("message_count")}
                  >
                    Messages{indicator("message_count")}
                  </button>
                </th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={`${styles.sortButton} rf-pressable`}
                    onClick={() => toggleSort("total_tokens")}
                  >
                    Total Tokens{indicator("total_tokens")}
                  </button>
                </th>
                <th className={styles.th}>Prompt</th>
                <th className={styles.th}>Completion</th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={`${styles.sortButton} rf-pressable`}
                    onClick={() => toggleSort("total_cost_usd")}
                  >
                    Cost{indicator("total_cost_usd")}
                  </button>
                </th>
              </tr>
            </thead>
            <tbody className="rf-stagger">
              {items.map((c) => (
                <tr key={c.id} className="rf-enter-rise">
                  <td className={styles.td}>{formatDate(c.updated_at)}</td>
                  <td className={`${styles.td} ${styles.titleCell}`}>
                    {c.title || c.id}
                  </td>
                  <td className={styles.td}>{c.model}</td>
                  <td className={styles.td}>{c.mode}</td>
                  <td className={styles.td}>{c.message_count}</td>
                  <td className={styles.td}>
                    {formatTokenCount(c.total_tokens ?? 0)}
                  </td>
                  <td className={styles.td}>
                    {formatTokenCount(c.total_prompt_tokens ?? 0)}
                  </td>
                  <td className={styles.td}>
                    {formatTokenCount(c.total_completion_tokens ?? 0)}
                  </td>
                  <td className={styles.td}>
                    {formatCostDisplay(c.total_cost_usd ?? null)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </Surface>
      )}
    </div>
  );
};

import React, { useEffect, useMemo, useState } from "react";
import { Badge, Surface } from "../../../components/ui";
import { useAppDispatch } from "../../../hooks";
import {
  trajectoriesApi,
  useListTrajectoriesPaginatedQuery,
} from "../../../services/refact/trajectories";
import type { TrajectoryMeta } from "../../../services/refact/trajectories";
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
  | "updated_at"
  | "total_lines_added";

type BadgeTone = React.ComponentProps<typeof Badge>["tone"];

type PagingState = {
  items: TrajectoryMeta[];
  isLoading: boolean;
  error: string | null;
};

function rangeCovered(
  rows: TrajectoryMeta[],
  from: string | undefined,
  hasMore: boolean,
) {
  if (!hasMore) return true;
  if (!from) return false;
  return rows.some((row) => row.updated_at < from);
}

function getProvider(model: string) {
  return model.includes("/") ? model.split("/")[0] : "—";
}

function getModelName(model: string) {
  return model.includes("/") ? model.split("/").slice(1).join("/") : model;
}

function getStatusTone(status: NonNullable<TrajectoryMeta["session_state"]>) {
  if (status === "completed") return "success" satisfies BadgeTone;
  if (status === "error") return "danger" satisfies BadgeTone;
  if (
    status === "paused" ||
    status === "waiting_ide" ||
    status === "waiting_user_input"
  ) {
    return "warning" satisfies BadgeTone;
  }
  return "default" satisfies BadgeTone;
}

function formatStatus(status: TrajectoryMeta["session_state"]) {
  return status ? status.replace(/_/g, " ") : "—";
}

export const ThreadsTab: React.FC<Props> = ({ dateRange }) => {
  const dispatch = useAppDispatch();
  const dateArgs = useMemo(() => dateRangeToApiArgs(dateRange), [dateRange]);
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
  const [paging, setPaging] = useState<PagingState>({
    items: [],
    isLoading: false,
    error: null,
  });

  useEffect(() => {
    if (!trajData) return;

    const firstPage = trajData;
    let cancelled = false;
    const isCancelled = () => cancelled;
    let activeRequest: { unsubscribe: () => void } | null = null;

    async function loadCoveredRange() {
      let rows = [...firstPage.items];
      let cursor = firstPage.next_cursor;
      let hasMore = firstPage.has_more;
      setPaging({
        items: [],
        isLoading: !rangeCovered(rows, dateArgs.from, hasMore),
        error: null,
      });

      while (
        !isCancelled() &&
        cursor &&
        hasMore &&
        !rangeCovered(rows, dateArgs.from, hasMore)
      ) {
        const request = dispatch(
          trajectoriesApi.endpoints.listTrajectoriesPaginated.initiate(
            { limit: 200, cursor },
            { forceRefetch: true, subscribe: false },
          ),
        );
        activeRequest = request;

        try {
          const result = await request.unwrap();
          request.unsubscribe();
          activeRequest = null;
          if (isCancelled()) return;

          rows = [...rows, ...result.items];
          cursor = result.next_cursor;
          hasMore = result.has_more;
          setPaging({
            items: rows.slice(firstPage.items.length),
            isLoading: !rangeCovered(rows, dateArgs.from, hasMore),
            error: null,
          });
        } catch (err) {
          request.unsubscribe();
          activeRequest = null;
          if (!isCancelled()) {
            setPaging({
              items: rows.slice(firstPage.items.length),
              isLoading: false,
              error:
                err instanceof Error ? err.message : "Failed to load threads",
            });
          }
          return;
        }
      }
    }

    void loadCoveredRange();

    return () => {
      cancelled = true;
      activeRequest?.unsubscribe();
    };
  }, [dispatch, trajData, dateArgs.from]);

  const rawItems = useMemo(() => {
    if (!trajData) return [];
    return [...trajData.items, ...paging.items];
  }, [trajData, paging.items]);

  const items = useMemo(() => {
    let rows = rawItems.filter((item) => {
      // `updated_at` is a full ISO timestamp; the bounds are calendar days, so
      // compare on the day prefix to keep the end day inclusive.
      const day = item.updated_at.slice(0, 10);
      if (dateArgs.from && day < dateArgs.from) return false;
      if (dateArgs.to && day > dateArgs.to) return false;
      return true;
    });
    const q = search.trim().toLowerCase();
    if (q) {
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
      } else if (sort.key === "total_lines_added") {
        av = a.total_lines_added;
        bv = b.total_lines_added;
      } else {
        av = a[sort.key] ?? 0;
        bv = b[sort.key] ?? 0;
      }
      if (av < bv) return sort.asc ? -1 : 1;
      if (av > bv) return sort.asc ? 1 : -1;
      return 0;
    });
    return rows;
  }, [rawItems, search, sort, dateArgs.from, dateArgs.to]);

  if (isLoading) return <Spinner spinning />;
  if (isError || paging.error) {
    return <ErrorCallout>Failed to load threads</ErrorCallout>;
  }

  if (!trajData || trajData.total_count === 0) {
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
        <p className={styles.emptyText}>
          {paging.isLoading ? "Loading more threads…" : "No matching threads."}
        </p>
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
                <th className={styles.th}>Provider</th>
                <th className={styles.th}>Mode</th>
                <th className={styles.th}>Status</th>
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
                <th className={styles.th}>Cache</th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={`${styles.sortButton} rf-pressable`}
                    onClick={() => toggleSort("total_cost_usd")}
                  >
                    Cost{indicator("total_cost_usd")}
                  </button>
                </th>
                <th className={styles.th}>
                  <button
                    type="button"
                    className={`${styles.sortButton} rf-pressable`}
                    onClick={() => toggleSort("total_lines_added")}
                  >
                    Lines{indicator("total_lines_added")}
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
                  <td className={styles.td}>{getModelName(c.model)}</td>
                  <td className={styles.td}>{getProvider(c.model)}</td>
                  <td className={styles.td}>{c.mode}</td>
                  <td className={styles.td}>
                    {c.session_state ? (
                      <Badge tone={getStatusTone(c.session_state)}>
                        {formatStatus(c.session_state)}
                      </Badge>
                    ) : (
                      <span className={styles.mutedText}>—</span>
                    )}
                  </td>
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
                    {formatTokenCount(c.total_cache_read_tokens ?? 0)}
                  </td>
                  <td className={styles.td}>
                    {formatCostDisplay(c.total_cost_usd ?? null)}
                  </td>
                  <td className={`${styles.td} ${styles.linesCell}`}>
                    {c.total_lines_added === 0 &&
                    c.total_lines_removed === 0 ? (
                      <span className={styles.mutedText}>—</span>
                    ) : (
                      <>
                        <span className={styles.linesAdded}>
                          +{c.total_lines_added}
                        </span>{" "}
                        /{" "}
                        <span className={styles.linesRemoved}>
                          -{c.total_lines_removed}
                        </span>
                      </>
                    )}
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

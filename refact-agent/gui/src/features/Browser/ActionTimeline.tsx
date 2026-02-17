import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import classNames from "classnames";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectTimeline,
  selectTimelineFilterSource,
  selectTimelineFilterType,
  setTimelineFilterSource,
  setTimelineFilterType,
  type TimelineEntry,
  type TimelineFilterSource,
} from "./browserSlice";
import styles from "./ActionTimeline.module.css";

type ActionTimelineProps = {
  chatId: string;
};

function formatTimestamp(ts: string): string {
  try {
    const date = new Date(ts);
    return date.toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  } catch {
    return ts;
  }
}

type TimelineEntryRowProps = {
  entry: TimelineEntry;
};

const TimelineEntryItem = ({ entry }: TimelineEntryRowProps) => {
  const [expanded, setExpanded] = useState(false);
  const hasDetails =
    entry.details !== undefined && Object.keys(entry.details).length > 0;

  const handleClick = useCallback(() => {
    if (hasDetails) {
      setExpanded((prev) => !prev);
    }
  }, [hasDetails]);

  return (
    <div
      className={styles.timelineEntry}
      onClick={handleClick}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          handleClick();
        }
      }}
      data-testid="timeline-entry"
    >
      <div className={styles.timelineEntryRow}>
        <span className={styles.entryTimestamp}>
          {formatTimestamp(entry.timestamp)}
        </span>
        <span
          className={classNames(styles.entryIcon, {
            [styles.entryIconUser]: entry.source === "user",
            [styles.entryIconAgent]: entry.source === "agent",
          })}
          aria-label={`Source: ${entry.source}`}
        >
          {entry.source === "user" ? "U" : "A"}
        </span>
        <span className={styles.entryType}>{entry.type}</span>
        <span className={styles.entrySummary} title={entry.summary}>
          {entry.summary}
        </span>
      </div>
      {expanded && hasDetails && (
        <div className={styles.entryDetails} data-testid="entry-details">
          {JSON.stringify(entry.details, null, 2)}
        </div>
      )}
    </div>
  );
};

const SOURCE_OPTIONS: { value: TimelineFilterSource; label: string }[] = [
  { value: "all", label: "All" },
  { value: "user", label: "User" },
  { value: "agent", label: "Agent" },
];

export const ActionTimeline = ({ chatId }: ActionTimelineProps) => {
  const dispatch = useAppDispatch();
  const timeline = useAppSelector((state) => selectTimeline(state, chatId));
  const filterSource = useAppSelector((state) =>
    selectTimelineFilterSource(state, chatId),
  );
  const filterType = useAppSelector((state) =>
    selectTimelineFilterType(state, chatId),
  );

  const listRef = useRef<HTMLDivElement>(null);

  const typeOptions = useMemo(() => {
    const types = new Set(timeline.map((e) => e.type));
    return Array.from(types).sort();
  }, [timeline]);

  const filtered = useMemo(() => {
    return timeline.filter((entry) => {
      if (filterSource !== "all" && entry.source !== filterSource) return false;
      if (filterType !== null && entry.type !== filterType) return false;
      return true;
    });
  }, [timeline, filterSource, filterType]);

  useEffect(() => {
    const el = listRef.current;
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
  }, [filtered.length]);

  const handleSourceFilter = useCallback(
    (source: TimelineFilterSource) => {
      dispatch(setTimelineFilterSource({ chatId, source }));
    },
    [dispatch, chatId],
  );

  const handleTypeFilter = useCallback(
    (type: string | null) => {
      dispatch(setTimelineFilterType({ chatId, type }));
    },
    [dispatch, chatId],
  );

  return (
    <div className={styles.timelineContainer} data-testid="action-timeline">
      <div className={styles.timelineHeader}>
        <span className={styles.timelineTitle}>Timeline</span>
        <div className={styles.filterGroup}>
          {SOURCE_OPTIONS.map((opt) => (
            <button
              key={opt.value}
              type="button"
              className={classNames(styles.filterButton, {
                [styles.filterButtonActive]: filterSource === opt.value,
              })}
              onClick={() => handleSourceFilter(opt.value)}
            >
              {opt.label}
            </button>
          ))}
          {typeOptions.length > 0 && (
            <>
              <button
                type="button"
                className={classNames(styles.filterButton, {
                  [styles.filterButtonActive]: filterType === null,
                })}
                onClick={() => handleTypeFilter(null)}
              >
                All types
              </button>
              {typeOptions.map((t) => (
                <button
                  key={t}
                  type="button"
                  className={classNames(styles.filterButton, {
                    [styles.filterButtonActive]: filterType === t,
                  })}
                  onClick={() => handleTypeFilter(t)}
                >
                  {t}
                </button>
              ))}
            </>
          )}
        </div>
      </div>
      <div className={styles.timelineList} ref={listRef}>
        {filtered.length === 0 ? (
          <div className={styles.emptyTimeline}>No timeline events</div>
        ) : (
          filtered.map((entry, idx) => (
            <TimelineEntryItem key={`${entry.timestamp}-${idx}`} entry={entry} />
          ))
        )}
      </div>
    </div>
  );
};

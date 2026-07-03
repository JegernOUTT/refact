import React, { useEffect, useMemo, useRef } from "react";
import { ArrowDownToLine, Eraser, Pause, Play } from "lucide-react";
import classNames from "classnames";

import {
  Chip,
  EmptyState,
  FieldText,
  IconButton,
  Tooltip,
} from "../../components/ui";
import { chipKeyHandler } from "./chipKeyHandler";
import { CopyButton } from "./CopyButton";
import {
  lineMatchesFilter,
  type BugReportSource,
  type LevelFilter,
} from "./useBugReportSources";
import styles from "./LogViewer.module.css";

const LEVEL_FILTERS: { value: LevelFilter; label: string }[] = [
  { value: "all", label: "ALL" },
  { value: "error", label: "ERR" },
  { value: "warn", label: "WARN" },
  { value: "info", label: "INFO" },
  { value: "debug", label: "DBG" },
];

export type LogViewerProps = {
  source: BugReportSource;
  filter: string;
  levelFilter: LevelFilter;
  paused: boolean;
  follow: boolean;
  onFilterChange: (value: string) => void;
  onLevelFilterChange: (value: LevelFilter) => void;
  onTogglePaused: () => void;
  onToggleFollow: () => void;
  onClear?: () => void;
};

export const LogViewer: React.FC<LogViewerProps> = ({
  source,
  filter,
  levelFilter,
  paused,
  follow,
  onFilterChange,
  onLevelFilterChange,
  onTogglePaused,
  onToggleFollow,
  onClear,
}) => {
  const viewRef = useRef<HTMLDivElement>(null);

  const visibleLines = useMemo(
    () =>
      source.lines.filter((line) =>
        lineMatchesFilter(line, filter, levelFilter),
      ),
    [source.lines, filter, levelFilter],
  );

  useEffect(() => {
    if (!follow) return;
    const view = viewRef.current;
    if (!view) return;
    view.scrollTop = view.scrollHeight;
  }, [visibleLines, follow]);

  const emptyDescription = !source.available
    ? "IDE logs aren't exposed by this host yet."
    : source.readError
      ? `Log file could not be read: ${source.readError}`
      : !source.exists
        ? "No log file found for this source."
        : filter || levelFilter !== "all"
          ? "No lines match the current filters."
          : "Waiting for log lines…";

  return (
    <div className={styles.panel}>
      <div className={styles.toolbar}>
        <div className={styles.search}>
          <FieldText
            aria-label="Filter log lines"
            onChange={onFilterChange}
            placeholder="Filter lines… (e.g. context, 401, codegraph)"
            value={filter}
          />
        </div>
        <div
          className={styles.levelChips}
          role="group"
          aria-label="Level filter"
        >
          {LEVEL_FILTERS.map(({ value, label }) => (
            <Chip
              key={value}
              className={styles.levelChip}
              onClick={() => onLevelFilterChange(value)}
              onKeyDown={chipKeyHandler(() => onLevelFilterChange(value))}
              radius="chip"
              role="button"
              selected={levelFilter === value}
              tabIndex={0}
            >
              {label}
            </Chip>
          ))}
        </div>
        <div className={styles.toolbarActions}>
          <Tooltip>
            <Tooltip.Trigger asChild>
              <IconButton
                aria-label={paused ? "Resume streaming" : "Pause streaming"}
                icon={paused ? Play : Pause}
                onClick={onTogglePaused}
                size="sm"
                variant="plain"
              />
            </Tooltip.Trigger>
            <Tooltip.Content side="bottom">
              {paused ? "Resume streaming" : "Pause streaming"}
            </Tooltip.Content>
          </Tooltip>
          <Tooltip>
            <Tooltip.Trigger asChild>
              <IconButton
                aria-label="Follow tail"
                className={classNames(follow && styles.actionActive)}
                icon={ArrowDownToLine}
                onClick={onToggleFollow}
                size="sm"
                variant="plain"
              />
            </Tooltip.Trigger>
            <Tooltip.Content side="bottom">
              {follow ? "Following tail" : "Follow tail"}
            </Tooltip.Content>
          </Tooltip>
          {onClear && (
            <Tooltip>
              <Tooltip.Trigger asChild>
                <IconButton
                  aria-label="Clear captured lines"
                  icon={Eraser}
                  onClick={onClear}
                  size="sm"
                  variant="plain"
                />
              </Tooltip.Trigger>
              <Tooltip.Content side="bottom">
                Clear captured lines
              </Tooltip.Content>
            </Tooltip>
          )}
        </div>
      </div>

      <div
        className={styles.view}
        ref={viewRef}
        data-testid="bug-report-log-view"
      >
        {visibleLines.length === 0 ? (
          <EmptyState description={emptyDescription} title="Nothing to show" />
        ) : (
          visibleLines.map((line, index) => (
            <div
              className={classNames(
                styles.line,
                line.level === "error" && styles.lineError,
                line.level === "warn" && styles.lineWarn,
              )}
              key={`${index}-${line.text.slice(0, 24)}`}
            >
              {line.text}
            </div>
          ))
        )}
      </div>

      <div className={styles.statusBar}>
        <span>{source.lines.length} lines</span>
        {source.path && (
          <>
            <span className={styles.statusSep}>·</span>
            <span
              className={classNames(styles.statusPath, "rf-truncate")}
              title={source.path}
            >
              {source.path}
            </span>
            <CopyButton label="Copy log path" text={source.path} />
          </>
        )}
        <span className={styles.statusSep}>·</span>
        <span className={paused ? styles.statusPaused : styles.statusLive}>
          {paused ? "paused" : "streaming"}
        </span>
        {source.readError && (
          <>
            <span className={styles.statusSep}>·</span>
            <span className={styles.statusPaused}>read error</span>
          </>
        )}
      </div>
    </div>
  );
};

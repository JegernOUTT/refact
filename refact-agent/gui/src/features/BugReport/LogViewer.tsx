import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
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

const BOTTOM_THRESHOLD_PX = 16;

function isScrolledToBottom(view: HTMLDivElement): boolean {
  const distanceFromBottom =
    view.scrollHeight - view.clientHeight - view.scrollTop;
  return distanceFromBottom <= BOTTOM_THRESHOLD_PX;
}

export type LogViewerProps = {
  source: BugReportSource;
  filter: string;
  levelFilter: LevelFilter;
  paused: boolean;
  onFilterChange: (value: string) => void;
  onLevelFilterChange: (value: LevelFilter) => void;
  onTogglePaused: () => void;
  onClear?: () => void;
};

export const LogViewer: React.FC<LogViewerProps> = ({
  source,
  filter,
  levelFilter,
  paused,
  onFilterChange,
  onLevelFilterChange,
  onTogglePaused,
  onClear,
}) => {
  const viewRef = useRef<HTMLDivElement>(null);
  const programmaticScrollRef = useRef(false);
  const previousSourceLengthRef = useRef(source.lines.length);
  const previousSourceKeyRef = useRef(source.key);
  const [follow, setFollow] = useState(true);
  const [newLineCount, setNewLineCount] = useState(0);

  const visibleLines = useMemo(
    () =>
      source.lines.filter((line) =>
        lineMatchesFilter(line, filter, levelFilter),
      ),
    [source.lines, filter, levelFilter],
  );

  const scrollToBottom = useCallback(() => {
    const view = viewRef.current;
    if (!view) return;
    if (view.scrollTop === view.scrollHeight) return;
    programmaticScrollRef.current = true;
    view.scrollTop = view.scrollHeight;
    window.setTimeout(() => {
      programmaticScrollRef.current = false;
    }, 0);
  }, []);

  useEffect(() => {
    if (previousSourceKeyRef.current !== source.key) {
      previousSourceKeyRef.current = source.key;
      previousSourceLengthRef.current = source.lines.length;
      setFollow(true);
      setNewLineCount(0);
      scrollToBottom();
    }
  }, [scrollToBottom, source.key, source.lines.length]);

  useEffect(() => {
    const previousLength = previousSourceLengthRef.current;
    const growth = Math.max(0, source.lines.length - previousLength);
    previousSourceLengthRef.current = source.lines.length;

    if (follow) {
      setNewLineCount(0);
      return;
    }

    if (growth > 0) {
      setNewLineCount((count) => Math.max(0, count + growth));
    }
  }, [follow, source.key, source.lines.length]);

  useEffect(() => {
    if (!follow) return;
    scrollToBottom();
  }, [visibleLines, follow, scrollToBottom]);

  const handleScroll = useCallback(() => {
    if (programmaticScrollRef.current) {
      programmaticScrollRef.current = false;
      return;
    }

    const view = viewRef.current;
    if (!view) return;

    if (isScrolledToBottom(view)) {
      previousSourceLengthRef.current = source.lines.length;
      setNewLineCount(0);
      setFollow(true);
      return;
    }

    setFollow(false);
  }, [source.lines.length]);

  const handleToggleFollow = useCallback(() => {
    if (follow) {
      setFollow(false);
      return;
    }

    previousSourceLengthRef.current = source.lines.length;
    setNewLineCount(0);
    setFollow(true);
    scrollToBottom();
  }, [follow, scrollToBottom, source.lines.length]);

  const handleJumpToLatest = useCallback(() => {
    previousSourceLengthRef.current = source.lines.length;
    setNewLineCount(0);
    setFollow(true);
    scrollToBottom();
  }, [scrollToBottom, source.lines.length]);

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
                aria-pressed={follow}
                className={classNames(follow && styles.actionActive)}
                icon={ArrowDownToLine}
                onClick={handleToggleFollow}
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

      <div className={styles.viewFrame}>
        <div
          className={styles.view}
          ref={viewRef}
          data-testid="bug-report-log-view"
          onScroll={handleScroll}
        >
          {visibleLines.length === 0 ? (
            <EmptyState
              description={emptyDescription}
              title="Nothing to show"
            />
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
        {!follow && (
          <button
            className={styles.latestPill}
            onClick={handleJumpToLatest}
            type="button"
          >
            {newLineCount > 0 ? `↓ ${newLineCount} new lines` : "↓ latest"}
          </button>
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

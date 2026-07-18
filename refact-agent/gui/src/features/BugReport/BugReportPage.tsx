import React, { useCallback, useState } from "react";
import { ArrowLeft, Bug, FolderSearch } from "lucide-react";

import {
  Badge,
  Icon,
  IconButton,
  Popover,
  StatusDot,
  Tabs,
  Tooltip,
} from "../../components/ui";
import { useConfig } from "../../hooks";
import { useGetBugReportContextQuery } from "../../services/refact/bugReport";
import { CopyButton } from "./CopyButton";
import { ErrorsPanel } from "./ErrorsPanel";
import { LogViewer } from "./LogViewer";
import { ReportForm } from "./ReportForm";
import {
  useBugReportSources,
  type BugReportSourceKey,
  type LevelFilter,
} from "./useBugReportSources";
import { clearWebuiLogEntries } from "./webuiLog";
import styles from "./BugReportPage.module.css";

export type BugReportPageProps = {
  onBack: () => void;
};

export const BugReportPage: React.FC<BugReportPageProps> = ({ onBack }) => {
  const config = useConfig();
  const [activeSource, setActiveSource] =
    useState<BugReportSourceKey>("engine");
  const [filter, setFilter] = useState("");
  const [levelFilter, setLevelFilter] = useState<LevelFilter>("all");
  const [paused, setPaused] = useState(false);

  const contextQuery = useGetBugReportContextQuery(undefined);
  const { sources, aggregatedErrors, webuiLines, ideLines } =
    useBugReportSources(paused, config.host);

  const active =
    sources.find((source) => source.key === activeSource) ?? sources[0];

  const handleJump = useCallback((source: BugReportSourceKey) => {
    setActiveSource(source);
    setFilter("");
    setLevelFilter("all");
  }, []);

  const handleTabChange = useCallback((value: string) => {
    setActiveSource(value as BugReportSourceKey);
  }, []);

  const context = contextQuery.data;
  const logPaths = context
    ? [
        { label: "Engine log", value: context.log_paths.engine_log_target },
        { label: "Daemon log", value: context.log_paths.daemon_log_file },
        { label: "Daemon logs dir", value: context.log_paths.daemon_logs_dir },
        { label: "Bundle folder", value: context.bundle_default_dir },
      ]
    : [];

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <IconButton
          aria-label="Back"
          icon={ArrowLeft}
          onClick={onBack}
          size="sm"
          variant="plain"
        />
        <span className={styles.bugBadge}>
          <Icon icon={Bug} size="md" tone="danger" />
        </span>
        <div className={styles.headerText}>
          <span className={styles.headerTitle}>Report a Bug</span>
          <span className={styles.headerSubtitle}>
            Live logs · aggregated errors · one-click GitHub issue
          </span>
        </div>
        <span className={styles.headerSpacer} />
        <Popover>
          <Tooltip>
            <Tooltip.Trigger asChild>
              <Popover.Trigger asChild>
                <IconButton
                  aria-label="Log locations"
                  icon={FolderSearch}
                  size="sm"
                  variant="plain"
                />
              </Popover.Trigger>
            </Tooltip.Trigger>
            <Tooltip.Content side="bottom">Log locations</Tooltip.Content>
          </Tooltip>
          <Popover.Content align="end" maxWidth="480px">
            <div className={styles.pathsList}>
              {logPaths.length === 0 && (
                <span className={styles.pathsEmpty}>
                  Waiting for engine context…
                </span>
              )}
              {logPaths.map((entry) => (
                <div className={styles.pathRow} key={entry.label}>
                  <span className={styles.pathLabel}>{entry.label}</span>
                  <span className={styles.pathValue} title={entry.value}>
                    {entry.value}
                  </span>
                  <CopyButton
                    label={`Copy ${entry.label}`}
                    text={entry.value}
                  />
                </div>
              ))}
            </div>
          </Popover.Content>
        </Popover>
        <span className={styles.streamChip}>
          <StatusDot status={paused ? "paused" : "running"} />
          {paused ? "streams paused" : "streams live"}
        </span>
      </header>

      <div className={styles.main}>
        <div className={styles.logsColumn}>
          <Tabs onValueChange={handleTabChange} value={active.key}>
            <Tabs.List aria-label="Log sources" className={styles.tabsList}>
              {sources.map((source) =>
                source.available ? (
                  <Tabs.Trigger key={source.key} value={source.key}>
                    <span className={styles.tabLabel}>
                      {source.label}
                      <span className={styles.tabCount}>
                        {source.lines.length}
                      </span>
                      {source.errorCount > 0 && (
                        <Badge size="xs" tone="danger">
                          {source.errorCount}
                        </Badge>
                      )}
                    </span>
                  </Tabs.Trigger>
                ) : (
                  <Tooltip key={source.key}>
                    <Tooltip.Trigger asChild>
                      <Tabs.Trigger disabled value={source.key}>
                        <span className={styles.tabLabel}>{source.label}</span>
                      </Tabs.Trigger>
                    </Tooltip.Trigger>
                    <Tooltip.Content side="bottom">
                      IDE logs aren&apos;t exposed by this host yet
                    </Tooltip.Content>
                  </Tooltip>
                ),
              )}
            </Tabs.List>
          </Tabs>
          <LogViewer
            filter={filter}
            levelFilter={levelFilter}
            onClear={active.key === "webui" ? clearWebuiLogEntries : undefined}
            onFilterChange={setFilter}
            onLevelFilterChange={setLevelFilter}
            onTogglePaused={() => setPaused((value) => !value)}
            paused={paused}
            source={active}
          />
        </div>

        <div className={styles.sideColumn}>
          <ErrorsPanel errors={aggregatedErrors} onJump={handleJump} />
          <ReportForm
            context={context}
            errors={aggregatedErrors}
            host={config.host}
            ideLines={ideLines}
            webuiLines={webuiLines}
          />
        </div>
      </div>
    </div>
  );
};

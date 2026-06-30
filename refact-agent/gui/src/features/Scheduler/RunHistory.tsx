import React, { useState } from "react";
import { ChevronDown } from "lucide-react";
import { Badge, Button, type BadgeTone } from "../../components/ui";
import type { CronRunRecord } from "../../services/refact/schedulerApi";
import styles from "./Scheduler.module.css";

type RunHistoryProps = {
  runs: CronRunRecord[];
};

function runTone(status: string): BadgeTone {
  const normalized = status.toLowerCase();
  if (["fired", "ok", "success"].includes(normalized)) return "success";
  if (normalized.includes("error") || normalized.includes("fail")) {
    return "danger";
  }
  if (normalized === "deferred" || normalized === "skipped") return "warning";
  return "muted";
}

function formatRunTime(timestampMs: number): string {
  return new Date(timestampMs).toLocaleString();
}

export const RunHistory: React.FC<RunHistoryProps> = ({ runs }) => {
  const [expanded, setExpanded] = useState(false);
  const sortedRuns = [...runs].sort((left, right) => right.at_ms - left.at_ms);

  if (sortedRuns.length === 0) {
    return (
      <section className={styles.runHistory} aria-label="Run history">
        <div className={styles.runHistoryHeader}>
          <span className={styles.runHistoryTitle}>Run history</span>
          <Badge tone="muted">No runs</Badge>
        </div>
      </section>
    );
  }

  return (
    <section className={styles.runHistory} aria-label="Run history">
      <Button
        className={styles.runHistoryToggle}
        type="button"
        variant="ghost"
        size="sm"
        aria-expanded={expanded}
        rightIcon={ChevronDown}
        onClick={() => setExpanded((value) => !value)}
      >
        Run history ({sortedRuns.length})
      </Button>
      {expanded ? (
        <ol className={styles.runHistoryList}>
          {sortedRuns.map((run) => {
            const runTime = formatRunTime(run.at_ms);
            return (
              <li
                className={styles.runHistoryItem}
                key={`${run.at_ms}-${run.status}`}
              >
                <div className={styles.runHistoryRow}>
                  <Badge tone={runTone(run.status)}>{run.status}</Badge>
                  <time dateTime={new Date(run.at_ms).toISOString()}>
                    {runTime}
                  </time>
                </div>
                {run.error ? (
                  <p className={styles.runHistoryError}>{run.error}</p>
                ) : null}
              </li>
            );
          })}
        </ol>
      ) : null}
    </section>
  );
};

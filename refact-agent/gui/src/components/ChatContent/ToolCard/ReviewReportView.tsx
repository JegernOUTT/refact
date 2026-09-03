import React, { useId, useMemo, useState } from "react";
import {
  Badge,
  Button,
  Chip,
  DataTable,
  type DataTableColumn,
  StatusDot,
  Surface,
} from "../../ui";
import { useOpenFileInApp } from "../../../hooks/useOpenFileInApp";
import {
  isHypothesis,
  severityOrder,
  severityTone,
  stageStatusLabel,
  stageStatusTone,
  type ReviewFinding,
  type ReviewReport,
  type ReviewSeverity,
  type StageRun,
} from "./reviewReportJson";
import styles from "./ReviewReportView.module.css";

const DEFAULT_FINDING_LIMIT = 12;

function locationLabel(finding: ReviewFinding): string {
  const file = finding.file || "Unknown file";
  if (finding.line_end !== finding.line_start) {
    return `${file}:${finding.line_start}-${finding.line_end}`;
  }
  return `${file}:${finding.line_start}`;
}

function humanizeDuration(durationMs: number): string {
  if (durationMs < 1000) return `${Math.round(durationMs)} ms`;
  const seconds = Math.round(durationMs / 1000);
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m${String(seconds % 60).padStart(
    2,
    "0",
  )}s`;
}

function middleTruncate(value: string, limit = 28): string {
  if (value.length <= limit) return value;
  const side = Math.floor((limit - 1) / 2);
  return `${value.slice(0, side)}…${value.slice(-side)}`;
}

function FindingCard({ finding }: { finding: ReviewFinding }) {
  const { canOpen, openFile } = useOpenFileInApp();
  const label = locationLabel(finding);
  const hasDetails =
    finding.evidence.length > 0 ||
    finding.fix !== null ||
    finding.locations.length > 0;

  return (
    <Surface as="li" className={styles.finding} variant="glass">
      <div className={styles.findingHeader}>
        <Badge size="xs" tone={severityTone(finding.severity)}>
          {finding.severity}
        </Badge>
        <Badge size="xs" tone="muted" variant="outline">
          {finding.reported_by.length > 0
            ? finding.reported_by.join("+")
            : finding.stage}
        </Badge>
        {finding.file.length > 0 ? (
          <button
            className={canOpen ? styles.fileLink : styles.filePlain}
            disabled={!canOpen}
            onClick={() =>
              openFile({ path: finding.file, line: finding.line_start })
            }
            type="button"
          >
            {label}
          </button>
        ) : (
          <span className={styles.filePlain}>{label}</span>
        )}
      </div>
      <p className={styles.claim}>{finding.claim || "No claim provided"}</p>
      <div className={styles.chipRow}>
        {finding.reproduction !== null && (
          <Chip>repro: {finding.reproduction}</Chip>
        )}
        {!finding.evidence_present && <Chip>evidence not found in file</Chip>}
        {!finding.introduced_by_diff && <Chip>pre-existing</Chip>}
        {finding.out_of_scope && <Chip>out of scope</Chip>}
        {finding.disputed !== null && (
          <Chip>disputed: {finding.disputed.reason}</Chip>
        )}
      </div>
      {hasDetails && (
        <details className={styles.details}>
          <summary>Details</summary>
          <div className={styles.detailsBody}>
            {finding.evidence.length > 0 && (
              <div>
                <span className={styles.detailLabel}>Evidence</span>
                <pre className={`${styles.evidenceContent} scrollX`}>
                  {finding.evidence}
                </pre>
              </div>
            )}
            {finding.fix !== null && (
              <div>
                <span className={styles.detailLabel}>Fix</span>
                <p>{finding.fix}</p>
              </div>
            )}
            {finding.locations.length > 0 && (
              <div>
                <span className={styles.detailLabel}>Also at</span>
                <div className={styles.chipRow}>
                  {finding.locations.map((location, index) => (
                    <Chip key={`${location.file}-${index}`}>
                      {`${location.file}:${location.line_start}-${location.line_end}`}
                    </Chip>
                  ))}
                </div>
              </div>
            )}
          </div>
        </details>
      )}
    </Surface>
  );
}

function FindingSection({
  findings,
  title,
}: {
  findings: ReviewFinding[];
  title: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const listId = useId();
  const visible = expanded
    ? findings
    : findings.slice(0, DEFAULT_FINDING_LIMIT);
  const hidden = findings.length - visible.length;

  return (
    <section className={styles.section}>
      <div className={styles.sectionHeader}>
        <h3 className={styles.sectionTitle}>{title}</h3>
        <Badge size="xs" tone="muted" variant="outline">
          {findings.length}
        </Badge>
      </div>
      <ul className={styles.findingList} id={listId}>
        {visible.map((finding, index) => (
          <FindingCard finding={finding} key={`${finding.id}-${index}`} />
        ))}
      </ul>
      {hidden > 0 && (
        <Button
          aria-controls={listId}
          onClick={() => setExpanded(true)}
          size="sm"
          variant="ghost"
        >
          Show {hidden} more
        </Button>
      )}
    </section>
  );
}

function stageDotStatus(status: StageRun["status"]) {
  if (status === "ok") return "success" as const;
  if (status === "failed") return "error" as const;
  if (status === "timed_out") return "warning" as const;
  return "idle" as const;
}

function stageIncompleteLabel(stage: StageRun): string {
  const status = stageStatusLabel(stage.status);
  const reason = stage.reason !== null ? `: ${stage.reason}` : "";
  return `${stage.name || "unnamed stage"} — ${status}${reason}`;
}

function OutcomeBanner({
  incomplete,
  report,
}: {
  incomplete: StageRun[];
  report: ReviewReport;
}) {
  if (report.outcome === "inconclusive") {
    return (
      <div
        className={styles.dangerCallout}
        data-testid="review-outcome-banner"
        role="alert"
      >
        <p className={styles.calloutTitle}>
          Review inconclusive — no stage completed and nothing was verified.
          This is not a pass.
        </p>
        {incomplete.length > 0 && (
          <ul className={styles.calloutList}>
            {incomplete.map((stage, index) => (
              <li key={`${stage.name}-${index}`}>
                {stageIncompleteLabel(stage)}
                {stage.trace_chat_id !== null && (
                  <>
                    {" "}
                    <span className={styles.mono}>
                      trace: {stage.trace_chat_id}
                    </span>
                  </>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>
    );
  }
  if (report.outcome === "partial") {
    const completed = report.stages.length - incomplete.length;
    return (
      <div
        className={styles.warningCallout}
        data-testid="review-outcome-banner"
        role="status"
      >
        <p className={styles.calloutTitle}>
          Partial review: {completed} of {report.stages.length} stages completed
          — findings may be incomplete.
        </p>
        {incomplete.length > 0 && (
          <ul className={styles.calloutList}>
            {incomplete.map((stage, index) => (
              <li key={`${stage.name}-${index}`}>
                {stageIncompleteLabel(stage)}
              </li>
            ))}
          </ul>
        )}
      </div>
    );
  }
  return null;
}

function DroppedFiles({ files }: { files: string[] }) {
  if (files.length === 0) return null;
  return (
    <details className={styles.details} data-testid="review-dropped-files">
      <summary>
        {files.length} file{files.length === 1 ? "" : "s"} were not reviewed
      </summary>
      <ul className={styles.droppedList}>
        {files.map((file, index) => (
          <li className={styles.mono} key={`${file}-${index}`}>
            {file}
          </li>
        ))}
      </ul>
    </details>
  );
}

function coverageLabel(stage: StageRun): string {
  const parts: string[] = [];
  if (stage.coverage.files_read.length > 0) {
    parts.push(`${stage.coverage.files_read.length} files`);
  }
  if (stage.coverage.commands_run.length > 0) {
    const failed = stage.coverage.commands_run.filter(
      (command) => command.exit !== 0,
    ).length;
    parts.push(`${stage.coverage.commands_run.length} cmds (${failed} failed)`);
  }
  if (stage.coverage.tools_unavailable.length > 0) {
    parts.push(`unavailable: ${stage.coverage.tools_unavailable.join(", ")}`);
  }
  if (stage.coverage.stopped_early !== null) {
    parts.push(`stopped: ${stage.coverage.stopped_early}`);
  }
  return parts.length > 0 ? parts.join(" · ") : "—";
}

const stageColumns: DataTableColumn<StageRun>[] = [
  { id: "stage", header: "Stage", cell: (row) => row.name || "—" },
  {
    id: "status",
    header: "Status",
    cell: (row) => (
      <span className={styles.status}>
        <StatusDot status={stageDotStatus(row.status)} />
        <Badge size="xs" tone={stageStatusTone(row.status)} variant="outline">
          {stageStatusLabel(row.status)}
        </Badge>
      </span>
    ),
  },
  {
    id: "reason",
    header: "Reason",
    cell: (row) => {
      const showTrace =
        row.trace_chat_id !== null &&
        (row.status === "failed" || row.status === "timed_out");
      if (!showTrace) return row.reason ?? "—";
      return (
        <span className={styles.reasonCell}>
          {row.reason ?? "—"}{" "}
          <span className={styles.mono}>trace: {row.trace_chat_id}</span>
        </span>
      );
    },
  },
  {
    id: "model",
    header: "Model",
    cell: (row) => (
      <span className={styles.model} title={row.model ?? undefined}>
        {row.model ? middleTruncate(row.model) : "—"}
      </span>
    ),
  },
  {
    id: "findings",
    header: "Findings",
    cell: (row) => row.findings,
    align: "end",
  },
  {
    id: "duration",
    header: "Duration",
    cell: (row) => humanizeDuration(row.duration_ms),
    align: "end",
  },
  { id: "coverage", header: "Coverage", cell: coverageLabel },
];

export const ReviewReportView: React.FC<{ report: ReviewReport }> = ({
  report,
}) => {
  const { facts, hypotheses, reproduced } = useMemo(() => {
    const facts = report.findings.filter((finding) => !isHypothesis(finding));
    return {
      facts,
      hypotheses: report.findings.filter(isHypothesis),
      reproduced: facts.filter((finding) => finding.reproduction !== null)
        .length,
    };
  }, [report.findings]);
  const bySeverity = useMemo(
    () =>
      severityOrder.map((severity: ReviewSeverity) => ({
        severity,
        findings: facts.filter((finding) => finding.severity === severity),
      })),
    [facts],
  );
  const incomplete = report.stages.filter((stage) => stage.status !== "ok");

  return (
    <div className={styles.report} data-testid="review-report">
      <OutcomeBanner incomplete={incomplete} report={report} />
      <Surface className={styles.header} variant="surface-2">
        <div className={styles.headerStrip}>
          <Badge tone="accent">{report.depth}</Badge>
          <span>
            {report.scope.requested_files} requested →{" "}
            {report.scope.reviewed_files} reviewed ({report.scope.mode})
          </span>
          {report.diff.base !== null && (
            <span className={styles.mono}>
              {report.diff.base}..{report.diff.head ?? "HEAD"} ·{" "}
              {report.diff.changed_files} files, {report.diff.hunks} hunks
            </span>
          )}
          <span>{humanizeDuration(report.duration_ms)}</span>
          {report.scope.focus !== null && (
            <span className={styles.truncated} title={report.scope.focus}>
              Focus: {report.scope.focus}
            </span>
          )}
        </div>
        <p className={styles.verdict}>
          {facts.length} supported ({reproduced} reproduced) ·{" "}
          {hypotheses.length} hypotheses · {report.duplicates_merged} duplicates
          merged · {report.scope.out_of_scope_findings} out of scope
        </p>
        {incomplete.length > 0 && report.outcome === null && (
          <p className={styles.summary}>
            Partial review: {incomplete.length} stage(s) did not complete —
            absence of findings there is not evidence of absence.
          </p>
        )}
        <DroppedFiles files={report.scope.dropped_files} />
      </Surface>

      {bySeverity
        .filter((group) => group.findings.length > 0)
        .map((group) => (
          <FindingSection
            findings={group.findings}
            key={group.severity}
            title={group.severity}
          />
        ))}

      {facts.length === 0 && report.outcome !== "inconclusive" && (
        <p className={styles.summary}>No supported findings.</p>
      )}

      {hypotheses.length > 0 && (
        <FindingSection
          findings={hypotheses}
          title="Hypotheses — unverified, you decide"
        />
      )}

      {report.stages.length > 0 && (
        <section className={styles.section}>
          <h3 className={styles.sectionTitle}>Stage coverage</h3>
          <DataTable
            caption="Stage coverage"
            columns={stageColumns}
            getRowId={(stage, index) => `${stage.name}-${index}`}
            rows={report.stages}
          />
        </section>
      )}

      {report.scratch_dir !== null && (
        <p className={styles.summary}>
          Raw per-stage output:{" "}
          <span className={styles.mono}>{report.scratch_dir}</span>
        </p>
      )}
    </div>
  );
};

export default ReviewReportView;

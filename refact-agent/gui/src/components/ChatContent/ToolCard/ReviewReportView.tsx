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
  agentStatusTone,
  severityTone,
  tierLabel,
  tierOrder,
  type ReviewAgentCoverage,
  type ReviewEvidence,
  type ReviewFinding,
  type ReviewRankTier,
  type ReviewReport,
} from "./reviewReportJson";
import styles from "./ReviewReportView.module.css";

const DEFAULT_FINDING_LIMIT = 12;
const CHECK_LIMIT = 30;

function locationLabel(
  path: string,
  line1: number | null,
  line2: number | null,
): string {
  if (line1 === null) return path || "Unknown location";
  return `${path || "Unknown file"}:${line1}${
    line2 !== null && line2 !== line1 ? `-${line2}` : ""
  }`;
}

function confidenceLabel(confidence: number): string {
  const percent = confidence <= 1 ? confidence * 100 : confidence;
  return `${Math.round(percent)}% confidence`;
}

function humanizeDuration(durationMs: number): string {
  if (durationMs < 1000) return `${Math.round(durationMs)} ms`;
  return `${(durationMs / 1000).toFixed(durationMs < 10_000 ? 1 : 0)} s`;
}

function middleTruncate(value: string, limit = 28): string {
  if (value.length <= limit) return value;
  const side = Math.floor((limit - 1) / 2);
  return `${value.slice(0, side)}…${value.slice(-side)}`;
}

function EvidenceRow({ evidence }: { evidence: ReviewEvidence }) {
  return (
    <li className={styles.evidenceRow}>
      <div className={styles.chipRow}>
        <Chip>{evidence.kind}</Chip>
        {evidence.path !== null && (
          <span className={styles.evidencePath} title={evidence.path}>
            {locationLabel(evidence.path, evidence.line1, evidence.line2)}
          </span>
        )}
      </div>
      {evidence.content.length > 0 && (
        <pre className={`${styles.evidenceContent} scrollX`}>
          {evidence.content}
        </pre>
      )}
    </li>
  );
}

function FindingCard({ finding }: { finding: ReviewFinding }) {
  const { canOpen, openFile } = useOpenFileInApp();
  const hasDetails =
    finding.impact !== null ||
    finding.remediation !== null ||
    finding.evidence.length > 0 ||
    finding.checks_performed.length > 0;
  const label = locationLabel(finding.file, finding.line1, finding.line2);

  return (
    <Surface as="li" className={styles.finding} variant="glass">
      <div className={styles.findingHeader}>
        <Badge size="xs" tone={severityTone(finding.severity)}>
          {finding.severity}
        </Badge>
        <Badge size="xs" tone="muted" variant="outline">
          {finding.category}
        </Badge>
        {finding.file.length > 0 ? (
          <button
            className={canOpen ? styles.fileLink : styles.filePlain}
            disabled={!canOpen}
            onClick={() =>
              openFile({ path: finding.file, line: finding.line1 ?? undefined })
            }
            type="button"
          >
            {label}
          </button>
        ) : (
          <span className={styles.filePlain}>{label}</span>
        )}
        <span className={styles.confidence}>
          {confidenceLabel(finding.confidence)}
        </span>
      </div>
      <p className={styles.claim}>{finding.claim || "No claim provided"}</p>
      {finding.sources.length > 0 && (
        <div className={styles.chipRow}>
          {finding.sources.map((source, index) => (
            <Chip key={`${source}-${index}`}>{source}</Chip>
          ))}
        </div>
      )}
      {hasDetails && (
        <details className={styles.details}>
          <summary>Details</summary>
          <div className={styles.detailsBody}>
            {finding.impact !== null && (
              <div>
                <span className={styles.detailLabel}>Impact</span>
                <p>{finding.impact}</p>
              </div>
            )}
            {finding.remediation !== null && (
              <div>
                <span className={styles.detailLabel}>Remediation</span>
                <p>{finding.remediation}</p>
              </div>
            )}
            {finding.evidence.length > 0 && (
              <div>
                <span className={styles.detailLabel}>Evidence</span>
                <ul className={styles.evidenceList}>
                  {finding.evidence.map((evidence, index) => (
                    <EvidenceRow
                      evidence={evidence}
                      key={`${evidence.kind}-${evidence.path ?? "none"}-${index}`}
                    />
                  ))}
                </ul>
              </div>
            )}
            {finding.checks_performed.length > 0 && (
              <div>
                <span className={styles.detailLabel}>Checks</span>
                <div className={styles.chipRow}>
                  {finding.checks_performed.map((check, index) => (
                    <Chip key={`${check}-${index}`}>{check}</Chip>
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
  tier,
}: {
  findings: ReviewFinding[];
  tier: ReviewRankTier;
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
        <h3 className={styles.sectionTitle}>{tierLabel(tier)}</h3>
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

function agentDotStatus(status: ReviewAgentCoverage["status"]) {
  if (status === "ran") return "success" as const;
  if (status === "failed") return "error" as const;
  return "idle" as const;
}

const agentColumns: DataTableColumn<ReviewAgentCoverage>[] = [
  { id: "agent", header: "Agent", cell: (row) => row.agent || "—" },
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
    id: "status",
    header: "Status",
    cell: (row) => (
      <span className={styles.status}>
        <StatusDot status={agentDotStatus(row.status)} />
        <Badge size="xs" tone={agentStatusTone(row.status)} variant="outline">
          {row.status}
        </Badge>
      </span>
    ),
  },
  { id: "reason", header: "Reason", cell: (row) => row.reason ?? "—" },
  {
    id: "candidates",
    header: "Candidates",
    cell: (row) => row.candidates,
    align: "end",
  },
  {
    id: "survived",
    header: "Survived",
    cell: (row) => row.survived,
    align: "end",
  },
  {
    id: "steps",
    header: "Steps",
    cell: (row) => row.steps ?? "—",
    align: "end",
  },
  {
    id: "duration",
    header: "Duration",
    cell: (row) => humanizeDuration(row.duration_ms),
    align: "end",
  },
];

export const ReviewReportView: React.FC<{ report: ReviewReport }> = ({
  report,
}) => {
  const grouped = useMemo(
    () =>
      tierOrder.map((tier) => ({
        tier,
        findings: report.findings.filter(
          (finding) => finding.rank_tier === tier,
        ),
      })),
    [report.findings],
  );
  const verdict = grouped
    .filter((group) => group.findings.length > 0)
    .map((group) => `${group.findings.length} ${tierLabel(group.tier)}`)
    .join(" · ");
  const mechanicalFailed =
    report.pipeline?.stopped_reason === "mechanical_checks_failed";
  const failedChecks = mechanicalFailed
    ? (report.pipeline?.mechanical?.checks.filter(
        (check) => check.exit_status !== 0,
      ) ?? [])
    : [];
  const shownChecks = report.checks_performed.slice(0, CHECK_LIMIT);
  const hiddenChecks = report.checks_performed.length - shownChecks.length;

  return (
    <div className={styles.report} data-testid="review-report">
      <Surface className={styles.header} variant="surface-2">
        <div className={styles.headerStrip}>
          {report.pipeline?.depth && (
            <Badge tone="accent">{report.pipeline.depth}</Badge>
          )}
          <span>{report.scope.files_reviewed.length} files reviewed</span>
          {report.scope.focus && (
            <span className={styles.truncated} title={report.scope.focus}>
              Focus: {report.scope.focus}
            </span>
          )}
          {report.scope.diff_base && (
            <span className={styles.mono}>Base: {report.scope.diff_base}</span>
          )}
        </div>
        <p className={styles.verdict}>
          Verdict: {verdict || "no findings"}
        </p>
        {report.summary && <p className={styles.summary}>{report.summary}</p>}
      </Surface>

      {mechanicalFailed && (
        <div className={styles.dangerCallout} role="alert">
          <strong>Mechanical checks failed</strong>
          {failedChecks.length === 0 && (
            <span>No failed check details were reported.</span>
          )}
          {failedChecks.map((check, index) => (
            <div className={styles.failedCheck} key={`${check.name}-${index}`}>
              <span>
                {check.name || "Check"} · exit {check.exit_status}
              </span>
              {check.output_excerpt && (
                <pre className={`${styles.failureOutput} scrollX`}>
                  {check.output_excerpt}
                </pre>
              )}
            </div>
          ))}
        </div>
      )}

      {report.assumed_intent && (
        <div className={styles.intent}>
          <strong>Assumed intent</strong>
          <span>{report.assumed_intent}</span>
        </div>
      )}

      {grouped
        .filter((group) => group.findings.length > 0)
        .map((group) => (
          <FindingSection
            findings={group.findings}
            key={group.tier}
            tier={group.tier}
          />
        ))}

      {report.pipeline && report.pipeline.agents.length > 0 && (
        <section className={styles.section}>
          <h3 className={styles.sectionTitle}>Agent coverage</h3>
          <DataTable
            caption="Agent coverage"
            columns={agentColumns}
            getRowId={(agent, index) => `${agent.agent}-${index}`}
            rows={report.pipeline.agents}
          />
        </section>
      )}

      {report.checks_performed.length > 0 && (
        <details className={styles.checks}>
          <summary>Checks performed ({report.checks_performed.length})</summary>
          <div className={styles.chipRow}>
            {shownChecks.map((check, index) => (
              <Chip key={`${check}-${index}`}>{check}</Chip>
            ))}
            {hiddenChecks > 0 && <Chip>+{hiddenChecks} more</Chip>}
          </div>
        </details>
      )}
    </div>
  );
};

export default ReviewReportView;

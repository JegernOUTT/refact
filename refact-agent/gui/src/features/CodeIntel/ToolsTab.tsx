import React, { useId, useMemo, useState } from "react";
import {
  FileCode2,
  GitBranch,
  ListChecks,
  Network,
  Radar,
  ShieldAlert,
  Users,
} from "lucide-react";

import {
  Badge,
  Button,
  Card,
  Chip,
  DataTable,
  EmptyState,
  ErrorState,
  FieldStack,
  FieldText,
  FieldTextarea,
  Icon,
  LoadingState,
} from "../../components/ui";
import type { BadgeTone, DataTableColumn } from "../../components/ui";
import {
  usePrBlastMutation,
  useSecurityScanMutation,
} from "../../services/refact/codeIntel";
import type {
  BlastImpact,
  BlastReport,
  CodeIntelDetail,
  CodeIntelResponse,
  SecurityFinding,
  Severity,
} from "../../services/refact/types";
import { StatCard } from "../StatsDashboard/components/StatCard";
import { formatNumber } from "../StatsDashboard/utils/formatters";
import styles from "./ToolsTab.module.css";

type RunStatus = "idle" | "loading" | "success" | "error";

type RunState<T> = {
  status: RunStatus;
  data?: T;
  detail?: string;
  error?: string;
};

type BlastReportWithReviewers = BlastReport & {
  suggested_reviewers?: string[];
  reviewers?: string[];
  reviewer_hints?: string[];
};

type ImpactRow = BlastImpact & {
  scope: "Direct" | "Transitive";
};

type ParsedMaxDepth =
  | { value?: number; error?: undefined }
  | { value?: undefined; error: string };

const initialBlastState: RunState<BlastReportWithReviewers> = {
  status: "idle",
};

const initialSecurityState: RunState<SecurityFinding[]> = {
  status: "idle",
};

const impactColumns: DataTableColumn<ImpactRow>[] = [
  {
    id: "scope",
    header: "Impact",
    cell: (row) => (
      <Badge tone={row.scope === "Direct" ? "accent" : "muted"}>
        {row.scope}
      </Badge>
    ),
    sortValue: (row) => row.distance,
  },
  {
    id: "path",
    header: "File",
    cell: (row) => <span className={styles.pathText}>{row.path}</span>,
    sortValue: (row) => row.path,
  },
  {
    id: "symbol",
    header: "Symbol",
    cell: (row) => row.symbol,
    sortValue: (row) => row.symbol,
  },
  {
    id: "distance",
    header: "Distance",
    cell: (row) => formatNumber(row.distance),
    sortValue: (row) => row.distance,
    align: "end",
  },
  {
    id: "via",
    header: "Via",
    cell: (row) => row.via,
    sortValue: (row) => row.via,
  },
];

const findingColumns: DataTableColumn<SecurityFinding>[] = [
  {
    id: "rule",
    header: "Rule",
    cell: (finding) => finding.rule,
    sortValue: (finding) => finding.rule,
  },
  {
    id: "severity",
    header: "Severity",
    cell: (finding) => <SeverityBadge severity={finding.severity} />,
    sortValue: (finding) => severityRank(finding.severity),
  },
  {
    id: "line",
    header: "Line",
    cell: (finding) => formatLine(finding.line),
    sortValue: (finding) => finding.line,
    align: "end",
  },
  {
    id: "snippet",
    header: "Snippet",
    cell: (finding) => (
      <code className={styles.snippet}>{finding.snippet || "—"}</code>
    ),
    sortValue: (finding) => finding.snippet,
  },
];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isCodeIntelDetail<T>(
  response: CodeIntelResponse<T>,
): response is CodeIntelDetail {
  return isRecord(response) && typeof response.detail === "string";
}

function describeMutationError(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;

  if (isRecord(error)) {
    const data = error.data;
    if (typeof data === "string") return data;
    if (isRecord(data) && typeof data.detail === "string") {
      return data.detail;
    }
    if (typeof error.error === "string") return error.error;
    if (typeof error.status === "number" || typeof error.status === "string") {
      return `Request failed with status ${error.status}.`;
    }
  }

  return "Request failed.";
}

function parseChangedFiles(value: string): string[] {
  return Array.from(
    new Set(
      value
        .split(/\r?\n|,/)
        .map((entry) => entry.trim())
        .filter(Boolean),
    ),
  );
}

function parseMaxDepth(value: string): ParsedMaxDepth {
  const trimmed = value.trim();
  if (!trimmed) return { value: undefined };

  const parsed = Number(trimmed);
  if (!Number.isInteger(parsed) || parsed < 1) {
    return { error: "Use a positive whole number." };
  }

  return { value: parsed };
}

function formatScore(score: number): string {
  if (!Number.isFinite(score)) return "—";
  if (Math.abs(score) >= 1) return score.toFixed(2);
  return score.toFixed(4);
}

function formatLine(line: number): string {
  if (!Number.isFinite(line) || line <= 0) return "—";
  return formatNumber(line);
}

function riskTone(score: number): "accent" | "success" | "warning" | "danger" {
  if (score >= 0.75) return "danger";
  if (score >= 0.4) return "warning";
  if (score > 0) return "success";
  return "accent";
}

function severityTone(severity: Severity): BadgeTone {
  if (severity === "Low") return "success";
  if (severity === "Medium") return "warning";
  return "danger";
}

function severityRank(severity: Severity): number {
  if (severity === "Critical") return 4;
  if (severity === "High") return 3;
  if (severity === "Medium") return 2;
  return 1;
}

function getReviewers(report: BlastReportWithReviewers): string[] {
  return (
    report.suggested_reviewers ??
    report.reviewers ??
    report.reviewer_hints ??
    []
  );
}

function SeverityBadge({ severity }: { severity: Severity }) {
  return (
    <Badge
      className={styles.severityBadge}
      data-severity={severity}
      tone={severityTone(severity)}
      variant="soft"
    >
      {severity}
    </Badge>
  );
}

function InlineDetail({ detail, title }: { detail: string; title: string }) {
  return (
    <div className={styles.stateSlot}>
      <EmptyState
        icon={Network}
        title={title}
        description={detail}
        variant="compact"
      />
    </div>
  );
}

function InlineLoading({ label }: { label: string }) {
  return (
    <div className={styles.stateSlot}>
      <LoadingState label={label} variant="compact" />
    </div>
  );
}

function InlineError({ error, title }: { error: string; title: string }) {
  return (
    <div className={styles.stateSlot}>
      <ErrorState title={title} description={error} variant="compact" />
    </div>
  );
}

function IdleState({
  description,
  title,
}: {
  description: string;
  title: string;
}) {
  return (
    <div className={styles.stateSlot}>
      <EmptyState title={title} description={description} variant="compact" />
    </div>
  );
}

function FileChips({ files }: { files: string[] }) {
  if (files.length === 0) {
    return <p className={styles.emptyText}>No files returned.</p>;
  }

  return (
    <div className={styles.chips}>
      {files.map((file) => (
        <Chip key={file} radius="chip">
          {file}
        </Chip>
      ))}
    </div>
  );
}

function BlastReportView({ report }: { report: BlastReportWithReviewers }) {
  const impactRows = useMemo<ImpactRow[]>(
    () => [
      ...report.directly_impacted.map((impact) => ({
        ...impact,
        scope: "Direct" as const,
      })),
      ...report.transitively_impacted.map((impact) => ({
        ...impact,
        scope: "Transitive" as const,
      })),
    ],
    [report.directly_impacted, report.transitively_impacted],
  );
  const reviewers = getReviewers(report);

  return (
    <div className={styles.resultStack}>
      <div className={styles.metricGrid}>
        <StatCard
          icon={FileCode2}
          title="Changed files"
          value={formatNumber(report.changed_files.length)}
          subtitle="paths submitted"
        />
        <StatCard
          icon={GitBranch}
          title="Directly impacted"
          value={formatNumber(report.directly_impacted.length)}
          subtitle="distance 1 symbols"
        />
        <StatCard
          icon={Radar}
          title="Transitively impacted"
          value={formatNumber(report.transitively_impacted.length)}
          subtitle="deeper dependency paths"
        />
        <StatCard
          icon={ListChecks}
          title="Impacted files"
          value={formatNumber(report.impacted_file_count)}
          subtitle="unique affected files"
        />
        <StatCard
          icon={ShieldAlert}
          title="Risk score"
          value={formatScore(report.risk_score)}
          subtitle="blast-radius score"
          tone={riskTone(report.risk_score)}
        />
      </div>

      <section
        className={styles.resultSection}
        aria-labelledby="changed-files-title"
      >
        <h4 className={styles.resultTitle} id="changed-files-title">
          Changed files
        </h4>
        <FileChips files={report.changed_files} />
      </section>

      {reviewers.length > 0 ? (
        <section
          className={styles.resultSection}
          aria-labelledby="suggested-reviewers-title"
        >
          <h4 className={styles.resultTitle} id="suggested-reviewers-title">
            Suggested reviewers
          </h4>
          <div className={styles.chips}>
            {reviewers.map((reviewer) => (
              <Chip
                key={reviewer}
                icon={<Icon icon={Users} size="sm" />}
                radius="chip"
              >
                {reviewer}
              </Chip>
            ))}
          </div>
        </section>
      ) : null}

      <section
        className={styles.resultSection}
        aria-labelledby="impacted-files-title"
      >
        <h4 className={styles.resultTitle} id="impacted-files-title">
          Impacted files
        </h4>
        {impactRows.length > 0 ? (
          <DataTable
            columns={impactColumns}
            rows={impactRows}
            getRowId={(row, index) =>
              `${row.scope}-${row.path}-${row.symbol}-${row.distance}-${index}`
            }
            enableSorting
            emptyMessage="No impacted files found."
            wide
          />
        ) : (
          <div className={styles.stateSlot}>
            <EmptyState
              icon={Radar}
              title="No impacted files found"
              description="The changed files did not produce a blast radius for the selected depth."
              variant="compact"
            />
          </div>
        )}
      </section>
    </div>
  );
}

function BlastPanel() {
  const changedFilesId = useId();
  const maxDepthId = useId();
  const [changedFilesText, setChangedFilesText] = useState("");
  const [maxDepthText, setMaxDepthText] = useState("");
  const [blastState, setBlastState] =
    useState<RunState<BlastReportWithReviewers>>(initialBlastState);
  const [runPrBlast] = usePrBlastMutation();

  const changedFiles = useMemo(
    () => parseChangedFiles(changedFilesText),
    [changedFilesText],
  );
  const maxDepth = useMemo(() => parseMaxDepth(maxDepthText), [maxDepthText]);
  const canRun =
    changedFiles.length > 0 &&
    !maxDepth.error &&
    blastState.status !== "loading";

  async function submitBlast() {
    if (!canRun) return;

    setBlastState({ status: "loading" });
    try {
      const response = await runPrBlast({
        changed_files: changedFiles,
        ...(maxDepth.value ? { max_depth: maxDepth.value } : {}),
      }).unwrap();
      if (isCodeIntelDetail(response)) {
        setBlastState({ status: "success", detail: response.detail });
      } else {
        setBlastState({ status: "success", data: response });
      }
    } catch (error) {
      setBlastState({ status: "error", error: describeMutationError(error) });
    }
  }

  function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    void submitBlast();
  }

  return (
    <Card
      aria-labelledby="pr-blast-title"
      className={styles.panel}
      padding="lg"
      role="region"
      variant="glass"
    >
      <div className={styles.panelHeader}>
        <span className={styles.iconShell}>
          <Icon icon={Radar} size="md" tone="accent" />
        </span>
        <div className={styles.panelCopy}>
          <h3 className={styles.panelTitle} id="pr-blast-title">
            PR Blast Radius
          </h3>
          <p className={styles.panelDescription}>
            Estimate reverse dependency impact from a set of changed files.
          </p>
        </div>
      </div>

      <form className={styles.form} onSubmit={handleSubmit}>
        <FieldStack
          htmlFor={changedFilesId}
          label="Changed files"
          helper="One path per line or comma-separated."
          required
        >
          <FieldTextarea
            id={changedFilesId}
            rows={4}
            value={changedFilesText}
            placeholder="src/main.rs\nsrc/router.ts"
            onChange={setChangedFilesText}
          />
        </FieldStack>

        {changedFiles.length > 0 ? (
          <div className={styles.parsedFiles} aria-label="Parsed changed files">
            <FileChips files={changedFiles} />
          </div>
        ) : null}

        <div className={styles.inlineControls}>
          <FieldStack
            className={styles.depthField}
            error={maxDepth.error}
            htmlFor={maxDepthId}
            label="Max depth"
            helper="Optional positive whole number."
          >
            <FieldText
              id={maxDepthId}
              inputMode="numeric"
              min={1}
              type="number"
              value={maxDepthText}
              placeholder="Default"
              onChange={setMaxDepthText}
            />
          </FieldStack>
          <Button
            className={styles.runButton}
            disabled={!canRun}
            loading={blastState.status === "loading"}
            type="submit"
            variant="primary"
          >
            Run
          </Button>
        </div>
      </form>

      {blastState.status === "idle" ? (
        <IdleState
          title="No blast run yet"
          description="Add changed files and run the blast-radius analyzer."
        />
      ) : blastState.status === "loading" ? (
        <InlineLoading label="Running PR blast analysis" />
      ) : blastState.status === "error" && blastState.error ? (
        <InlineError title="PR blast failed" error={blastState.error} />
      ) : blastState.detail ? (
        <InlineDetail
          title="CodeGraph is unavailable"
          detail={blastState.detail}
        />
      ) : blastState.data ? (
        <BlastReportView report={blastState.data} />
      ) : null}
    </Card>
  );
}

function SecurityPanel() {
  const pathId = useId();
  const [path, setPath] = useState("");
  const [securityState, setSecurityState] =
    useState<RunState<SecurityFinding[]>>(initialSecurityState);
  const [runSecurityScan] = useSecurityScanMutation();
  const trimmedPath = path.trim();
  const canRun = trimmedPath.length > 0 && securityState.status !== "loading";

  async function submitSecurityScan() {
    if (!canRun) return;

    setSecurityState({ status: "loading" });
    try {
      const response = await runSecurityScan({ path: trimmedPath }).unwrap();
      if (isCodeIntelDetail(response)) {
        setSecurityState({ status: "success", detail: response.detail });
      } else {
        setSecurityState({ status: "success", data: response });
      }
    } catch (error) {
      setSecurityState({
        status: "error",
        error: describeMutationError(error),
      });
    }
  }

  function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    void submitSecurityScan();
  }

  return (
    <Card
      aria-labelledby="security-scan-title"
      className={styles.panel}
      padding="lg"
      role="region"
      variant="glass"
    >
      <div className={styles.panelHeader}>
        <span className={styles.iconShell}>
          <Icon icon={ShieldAlert} size="md" tone="accent" />
        </span>
        <div className={styles.panelCopy}>
          <h3 className={styles.panelTitle} id="security-scan-title">
            Security Scan
          </h3>
          <p className={styles.panelDescription}>
            Scan a file for secrets, injection, eval, TLS, crypto, and random
            risks.
          </p>
        </div>
      </div>

      <form className={styles.form} onSubmit={handleSubmit}>
        <div className={styles.inlineControls}>
          <FieldStack
            className={styles.pathField}
            htmlFor={pathId}
            label="Path"
            helper="Workspace-relative or indexed source path."
            required
          >
            <FieldText
              id={pathId}
              value={path}
              placeholder="src/server.ts"
              onChange={setPath}
            />
          </FieldStack>
          <Button
            className={styles.runButton}
            disabled={!canRun}
            loading={securityState.status === "loading"}
            type="submit"
            variant="primary"
          >
            Scan
          </Button>
        </div>
      </form>

      {securityState.status === "idle" ? (
        <IdleState
          title="No security scan yet"
          description="Enter a file path and run the scanner."
        />
      ) : securityState.status === "loading" ? (
        <InlineLoading label="Running security scan" />
      ) : securityState.status === "error" && securityState.error ? (
        <InlineError title="Security scan failed" error={securityState.error} />
      ) : securityState.detail ? (
        <InlineDetail
          title="CodeGraph is unavailable"
          detail={securityState.detail}
        />
      ) : securityState.data && securityState.data.length > 0 ? (
        <DataTable
          columns={findingColumns}
          rows={securityState.data}
          getRowId={(finding, index) =>
            `${finding.rule}-${finding.line}-${index}`
          }
          enableSorting
          emptyMessage="No security findings."
          wide
        />
      ) : securityState.data ? (
        <div className={styles.stateSlot}>
          <EmptyState
            icon={ShieldAlert}
            title="No security findings"
            description="The scanner did not report findings for this file."
            variant="compact"
          />
        </div>
      ) : null}
    </Card>
  );
}

export function ToolsTab() {
  return (
    <div className={styles.root}>
      <BlastPanel />
      <SecurityPanel />
    </div>
  );
}

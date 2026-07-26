import React, { useId, useMemo, useState } from "react";
import classNames from "classnames";
import { Badge, Button } from "../../ui";
import {
  shortenPath,
  type AnalysisMetric,
  type AnalysisReport,
  type AnalysisRow,
  type AnalysisSection,
  type AnalysisSeverity,
} from "./engineAnalysisJson";
import styles from "./AnalysisReport.module.css";

const DEFAULT_ROW_LIMIT = 12;

function severityTone(
  severity: AnalysisSeverity,
): React.ComponentProps<typeof Badge>["tone"] {
  if (severity === "Critical" || severity === "High") return "danger";
  if (severity === "Medium") return "warning";
  return "success";
}

function MetricGrid({ metrics }: { metrics: AnalysisMetric[] }) {
  return (
    <div className={styles.metricGrid}>
      {metrics.map((metric, index) => (
        <div className={styles.metricCard} key={`${metric.key}-${index}`}>
          <span className={styles.metricLabel}>{metric.key}</span>
          <span className={styles.metricValue}>{metric.value}</span>
        </div>
      ))}
    </div>
  );
}

function ReportRow({
  prefix,
  row,
}: {
  prefix: string | null;
  row: AnalysisRow;
}) {
  const hasTitle = row.title.length > 0;
  const bodyPaths = hasTitle ? row.paths : [];

  return (
    <li className={styles.row}>
      <div className={styles.rowMain}>
        {row.lead !== null && <span className={styles.lead}>{row.lead}</span>}
        {row.severity !== null && (
          <Badge tone={severityTone(row.severity)} variant="soft">
            {row.severityLabel ?? row.severity}
          </Badge>
        )}
        {hasTitle ? (
          <span className={styles.rowTitle}>{row.title}</span>
        ) : (
          <span
            className={classNames(styles.rowTitle, styles.rowTitleMono)}
            title={row.paths.length > 0 ? row.paths.join("  ↔  ") : undefined}
          >
            {row.paths.length > 0
              ? row.paths
                  .map((path) => shortenPath(path, prefix))
                  .join("  ↔  ")
              : row.raw}
          </span>
        )}
        {row.tags.map((tag) => (
          <Badge key={tag} tone="warning" variant="outline">
            {tag}
          </Badge>
        ))}
      </div>
      {bodyPaths.length > 0 && (
        <div className={styles.chipRow}>
          {bodyPaths.map((path, index) => (
            <span className={styles.path} key={`${path}-${index}`} title={path}>
              {shortenPath(path, prefix)}
            </span>
          ))}
        </div>
      )}
      {row.detail !== null && (
        <span className={styles.detail}>{row.detail}</span>
      )}
      {row.metrics.length > 0 && (
        <div className={styles.chipRow}>
          {row.metrics.map((metric, index) => (
            <Badge
              key={`${metric.key}-${index}`}
              title={metric.value}
              tone="muted"
              variant="outline"
            >
              {metric.key} {shortenPath(metric.value, prefix)}
            </Badge>
          ))}
        </div>
      )}
    </li>
  );
}

function ReportSection({
  prefix,
  section,
}: {
  prefix: string | null;
  section: AnalysisSection;
}) {
  const [expanded, setExpanded] = useState(false);
  const listId = useId();
  const rows = expanded
    ? section.rows
    : section.rows.slice(0, DEFAULT_ROW_LIMIT);
  const hidden = section.rows.length - rows.length;
  const label = section.title.length > 0 ? section.title : "this group";

  return (
    <div className={styles.section}>
      {section.title.length > 0 && (
        <div className={styles.sectionHeader}>
          <span
            className={classNames(
              styles.sectionTitle,
              section.titleIsPath && styles.sectionTitlePath,
            )}
            title={section.titleIsPath ? section.title : undefined}
          >
            {section.titleIsPath
              ? shortenPath(section.title, prefix)
              : section.title}
          </span>
          <Badge tone="muted" variant="outline">
            {section.rows.length}
          </Badge>
        </div>
      )}
      {section.metrics ? (
        <MetricGrid metrics={section.metrics} />
      ) : (
        <ul className={styles.rowList} id={listId}>
          {rows.map((row) => (
            <ReportRow key={row.line} prefix={prefix} row={row} />
          ))}
        </ul>
      )}
      {hidden > 0 && (
        <div className={styles.controlRow}>
          <Button
            aria-controls={listId}
            onClick={() => setExpanded(true)}
            size="sm"
            variant="ghost"
          >
            Show {hidden} more in {label}
          </Button>
        </div>
      )}
    </div>
  );
}

export const AnalysisReportView: React.FC<{ report: AnalysisReport }> = ({
  report,
}) => {
  const indexChips = useMemo(
    () =>
      report.indexState.map((metric) => ({
        ...metric,
        tone:
          metric.key === "partial" && metric.value === "true"
            ? ("warning" as const)
            : ("muted" as const),
      })),
    [report.indexState],
  );

  return (
    <div className={styles.report} data-testid="analysis-report">
      {report.warnings.map((warning) => (
        <div className={styles.warning} key={warning}>
          {warning}
        </div>
      ))}
      {report.headline !== null && (
        <p className={styles.headline}>{report.headline}</p>
      )}
      {report.facts.length > 0 && <MetricGrid metrics={report.facts} />}
      {indexChips.length > 0 ? (
        <div className={styles.chipRow} title={report.indexStateRaw ?? ""}>
          {indexChips.map((metric) => (
            <Badge key={metric.key} tone={metric.tone} variant="outline">
              {metric.key} {metric.value}
            </Badge>
          ))}
        </div>
      ) : (
        report.indexStateRaw !== null && (
          <p className={styles.prefixNote}>{report.indexStateRaw}</p>
        )
      )}
      {report.sections.map((section) => (
        <ReportSection
          key={section.line}
          prefix={report.pathPrefix}
          section={section}
        />
      ))}
      {report.pathPrefix !== null && (
        <p className={styles.prefixNote}>
          paths relative to {report.pathPrefix}
        </p>
      )}
    </div>
  );
};

export default AnalysisReportView;

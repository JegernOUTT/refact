import {
  compactText,
  type CompactAgenticPresentation,
} from "../../Tools/AgenticTool/compactPresentation";
import {
  isHypothesis,
  severityOrder,
  type ReviewReport,
} from "./reviewReportJson";

export function reviewPresentation(
  report: ReviewReport | null,
  hasResult: boolean,
  failed: boolean,
): CompactAgenticPresentation {
  if (!report) {
    return {
      headline: "Inspect code changes",
      stateLabel: failed
        ? "Request failed"
        : hasResult
          ? "Review unavailable"
          : "Awaiting review",
      tone: failed ? "danger" : hasResult ? "warning" : "active",
      facts: [{ label: "Coverage", value: "No structured review evidence" }],
    };
  }
  const completed = report.stages.filter(
    (stage) => stage.status === "ok",
  ).length;
  const unavailable =
    report.outcome === "inconclusive" ||
    report.stages.length === 0 ||
    completed === 0;
  const partial =
    report.outcome === "partial" ||
    completed !== report.stages.length ||
    report.scope.dropped_files.length > 0 ||
    report.scope.reviewed_files < report.scope.requested_files ||
    report.stages.some(
      (stage) =>
        stage.coverage.stopped_early !== null ||
        stage.coverage.tools_unavailable.length > 0,
    );
  const supported = report.findings.filter((finding) => !isHypothesis(finding));
  const severity = severityOrder
    .map((level) => {
      const count = supported.filter(
        (finding) => finding.severity === level,
      ).length;
      return count > 0 ? `${count} ${level}` : null;
    })
    .filter(Boolean)
    .join(" · ");
  const count = report.findings.length;
  return {
    headline:
      compactText(
        unavailable
          ? `${report.scope.requested_files} files in scope${
              count > 0
                ? ` · ${count} finding${count === 1 ? "" : "s"} reported`
                : ""
            }`
          : `${report.scope.reviewed_files} files reviewed · ${count} finding${
              count === 1 ? "" : "s"
            }`,
        160,
      ) ?? "Code changes inspected",
    stateLabel: failed
      ? "Request failed"
      : unavailable
        ? "Review unavailable"
        : partial
          ? "Partial review"
          : count > 0
            ? "Findings reported"
            : "0 findings reported",
    tone: failed
      ? "danger"
      : unavailable || partial || count > 0
        ? "warning"
        : "neutral",
    facts: [
      {
        label: "Findings",
        value: `${supported.length} supported · ${
          count - supported.length
        } hypotheses (unverified)`,
      },
      {
        label: "Severity",
        value: severity || "No supported findings; not a verification pass",
      },
      {
        label: "Coverage",
        value: `${completed}/${report.stages.length} stages completed · ${
          unavailable
            ? `${report.scope.requested_files} files in scope`
            : `${report.scope.reviewed_files}/${report.scope.requested_files} files reviewed`
        } · ${report.scope.dropped_files.length} dropped`,
      },
    ],
  };
}

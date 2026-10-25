import {
  compactText,
  readCompactRecord,
  type CompactAgenticPresentation,
} from "../Tools/AgenticTool/compactPresentation";

export type VerificationResult = {
  command: string;
  exit_code?: number | null;
  passed: boolean;
  output_tail: string;
};
export type SuggestedCard = {
  title: string;
  priority: string;
  instructions: string;
};
export type FinalReport = {
  summary: string;
  success: boolean;
  files_changed: string[];
  tests_added_or_updated: string[];
  verification: VerificationResult[];
  followup_cards: SuggestedCard[];
  risks: string[];
  assumptions: string[];
};

function optionalStringArray(value: unknown): string[] | null {
  if (value === undefined || value === null) return [];
  if (!Array.isArray(value)) return null;
  return value.every((item) => typeof item === "string") ? value : null;
}

function parseVerification(value: unknown): VerificationResult[] | null {
  if (value === undefined || value === null) return [];
  if (!Array.isArray(value)) return null;
  const out: VerificationResult[] = [];
  for (const item of value) {
    const record = readCompactRecord(item);
    if (!record) return null;
    const exitCode = record.exit_code;
    if (
      typeof record.command !== "string" ||
      typeof record.passed !== "boolean" ||
      typeof record.output_tail !== "string"
    )
      return null;
    if (
      exitCode !== undefined &&
      exitCode !== null &&
      (typeof exitCode !== "number" || !Number.isFinite(exitCode))
    )
      return null;
    out.push({
      command: record.command,
      passed: record.passed,
      output_tail: record.output_tail,
      exit_code: exitCode,
    });
  }
  return out;
}

function parseFollowups(value: unknown): SuggestedCard[] | null {
  if (value === undefined || value === null) return [];
  if (!Array.isArray(value)) return null;
  const out: SuggestedCard[] = [];
  for (const item of value) {
    const record = readCompactRecord(item);
    if (
      !record ||
      typeof record.title !== "string" ||
      typeof record.priority !== "string" ||
      typeof record.instructions !== "string"
    )
      return null;
    out.push({
      title: record.title,
      priority: record.priority,
      instructions: record.instructions,
    });
  }
  return out;
}

export function parseFinalReport(content: string): FinalReport | null {
  const raw = readCompactRecord(content);
  if (
    !raw ||
    typeof raw.summary !== "string" ||
    typeof raw.success !== "boolean"
  )
    return null;
  const files = optionalStringArray(raw.files_changed);
  const tests = optionalStringArray(raw.tests_added_or_updated);
  const verification = parseVerification(raw.verification);
  const followups = parseFollowups(raw.followup_cards);
  const risks = optionalStringArray(raw.risks);
  const assumptions = optionalStringArray(raw.assumptions);
  if (!files || !tests || !verification || !followups || !risks || !assumptions)
    return null;
  return {
    summary: raw.summary,
    success: raw.success,
    files_changed: files,
    tests_added_or_updated: tests,
    verification,
    followup_cards: followups,
    risks,
    assumptions,
  };
}

export function verificationSummary(report: FinalReport): string {
  if (report.verification.length === 0) return "No verification evidence";
  const passed = report.verification.filter(verificationPassed).length;
  return `${passed}/${report.verification.length} checks reported passed`;
}

export function verificationPassed(item: VerificationResult): boolean {
  return item.passed && item.exit_code === 0 && item.command.trim() !== "";
}

export function finalReportPresentation(
  content: string | null,
  toolFailed = false,
): CompactAgenticPresentation {
  const report = content ? parseFinalReport(content) : null;
  return {
    headline: compactText(report?.summary, 160) ?? "Task agent final report",
    stateLabel: toolFailed
      ? "Request failed"
      : content === null
        ? "Awaiting report"
        : report
          ? report.success
            ? "Reported success"
            : "Reported failure"
          : "Outcome unconfirmed",
    tone:
      toolFailed || report?.success === false
        ? "danger"
        : content === null
          ? "active"
          : "neutral",
    facts: report
      ? [
          {
            label: "Reported outcome",
            value: report.success
              ? "Success (self-reported)"
              : "Failure (self-reported)",
          },
          { label: "Verification", value: verificationSummary(report) },
          {
            label: "Changes",
            value: `${report.files_changed.length} files · ${report.tests_added_or_updated.length} tests added or updated`,
          },
        ]
      : [
          {
            label: "Evidence",
            value:
              content === null
                ? "No report received"
                : "No structured outcome or verification evidence",
          },
        ],
  };
}

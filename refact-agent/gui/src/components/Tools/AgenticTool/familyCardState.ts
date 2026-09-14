import type { AgenticToolViewModel } from "./agenticToolModel";
import { reportIsStructured, type FamilyReport } from "./familyReport";

export const SUMMARY_CHARS = 240;
const SUMMARY_LINES = 2;

export type FamilyCardState =
  | "pending"
  | "failed"
  | "malformed"
  | "legacy"
  | "ready";

export type FamilyCardTexts = {
  ready: string;
  pending: string;
  failed: string;
  malformed: string;
  legacy: string;
  note: string;
};

export type FamilyViewProps = {
  model: AgenticToolViewModel;
  content: string | null;
};

export function isSummaryBody(body: string | null): body is string {
  return (
    body !== null &&
    body.length <= SUMMARY_CHARS &&
    body.split("\n").length <= SUMMARY_LINES
  );
}

export function familyCardState(
  model: AgenticToolViewModel,
  report: FamilyReport,
): FamilyCardState {
  if (model.status === "pending") return "pending";
  if (model.status === "failed") return "failed";
  if (model.malformedArgs || report.kind === "malformed") return "malformed";
  if (
    reportIsStructured(report) ||
    model.details.length > 0 ||
    isSummaryBody(model.body)
  )
    return "ready";
  return "legacy";
}

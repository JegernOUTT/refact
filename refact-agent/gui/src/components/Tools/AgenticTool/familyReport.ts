import type { AgenticDetailRow } from "./agenticToolModel";

export const REPORT_VALUE_CHARS = 240;
export const REPORT_MAX_ROWS = 8;
export const REPORT_MAX_ITEMS = 8;
export const REPORT_MAX_FIELDS = 12;

export type ReportRow = { key: string; cells: string[] };

export type FamilyReport = {
  kind: "markdown" | "json" | "malformed" | "empty";
  title: string | null;
  headers: string[];
  rows: ReportRow[];
  rowTotal: number;
  items: string[];
  itemTotal: number;
  fields: AgenticDetailRow[];
  fieldTotal: number;
};

const EMPTY_REPORT: FamilyReport = {
  kind: "empty",
  title: null,
  headers: [],
  rows: [],
  rowTotal: 0,
  items: [],
  itemTotal: 0,
  fields: [],
  fieldTotal: 0,
};

const DETAIL_LINE = /^[^:]{1,40}:\s*.+$/u;
const TABLE_RULE = /^:?-{2,}:?$/u;

function clip(value: string): string {
  const normalized = value.replace(/\s+/gu, " ").trim();
  if (normalized.length <= REPORT_VALUE_CHARS) return normalized;
  return `${normalized.slice(0, REPORT_VALUE_CHARS - 1)}…`;
}

function fieldText(value: unknown): string {
  if (typeof value === "string") return clip(value);
  if (typeof value === "number" || typeof value === "boolean")
    return String(value);
  if (value === null) return "none";
  if (Array.isArray(value)) return `${value.length} items`;
  return clip(JSON.stringify(value));
}

function jsonReport(value: unknown): FamilyReport {
  if (Array.isArray(value)) {
    return {
      ...EMPTY_REPORT,
      kind: "json",
      headers: ["Item"],
      rows: value.slice(0, REPORT_MAX_ROWS).map((item, index) => ({
        key: `item-${index}`,
        cells: [fieldText(item)],
      })),
      rowTotal: value.length,
    };
  }
  if (typeof value !== "object" || value === null) {
    return { ...EMPTY_REPORT, kind: "malformed" };
  }
  const entries = Object.entries(value as Record<string, unknown>);
  return {
    ...EMPTY_REPORT,
    kind: "json",
    fields: entries.slice(0, REPORT_MAX_FIELDS).map(([label, item]) => ({
      label: clip(label),
      value: fieldText(item),
    })),
    fieldTotal: entries.length,
  };
}

function markdownReport(content: string): FamilyReport {
  let title: string | null = null;
  let headers: string[] = [];
  const rows: ReportRow[] = [];
  let rowTotal = 0;
  const items: string[] = [];
  let itemTotal = 0;
  for (const line of content.split("\n")) {
    const trimmed = line.trim();
    if (trimmed === "") continue;
    if (trimmed.startsWith("#")) {
      const heading = clip(trimmed.replace(/^#+/u, ""));
      if (title === null && heading !== "") title = heading;
      continue;
    }
    if (trimmed.startsWith("|") && trimmed.endsWith("|")) {
      const cells = trimmed
        .slice(1, -1)
        .split("|")
        .map((cell) => clip(cell));
      if (cells.every((cell) => TABLE_RULE.test(cell))) continue;
      if (headers.length === 0) {
        headers = cells;
        continue;
      }
      rowTotal += 1;
      if (rows.length < REPORT_MAX_ROWS) {
        rows.push({ key: `row-${rowTotal}`, cells });
      }
      continue;
    }
    if (trimmed.startsWith("- ")) {
      const item = clip(trimmed.slice(2));
      if (item === "" || DETAIL_LINE.test(item)) continue;
      itemTotal += 1;
      if (items.length < REPORT_MAX_ITEMS) items.push(item);
    }
  }
  return {
    ...EMPTY_REPORT,
    kind: "markdown",
    title,
    headers,
    rows,
    rowTotal,
    items,
    itemTotal,
  };
}

export function parseFamilyReport(content: string | null): FamilyReport {
  const text = (content ?? "").trim();
  if (text === "") return EMPTY_REPORT;
  if (text.startsWith("{") || text.startsWith("[")) {
    try {
      return jsonReport(JSON.parse(text) as unknown);
    } catch {
      return { ...EMPTY_REPORT, kind: "malformed" };
    }
  }
  return markdownReport(text);
}

export function reportIsStructured(report: FamilyReport): boolean {
  return (
    report.title !== null ||
    report.rows.length > 0 ||
    report.items.length > 0 ||
    report.fields.length > 0
  );
}

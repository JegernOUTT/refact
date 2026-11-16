/**
 * Pure helpers for what a script showed (tables, charts, markdown) and for
 * the model calls it made. Inputs are untrusted engine values: everything
 * returns plain text or plain data, never HTML.
 */
import {
  AreaChart,
  BarChart3,
  FileText,
  LineChart,
  PieChart,
  ScatterChart,
  Shapes,
  Table2,
  type LucideIcon,
} from "lucide-react";
import type {
  PythonCallRecord,
  PythonDisplay,
  PythonDisplayCell,
} from "../../../services/refact/types";
import { isInsideDir } from "./pythonDisplay";
export const CHART_KINDS = ["bar", "line", "area", "scatter", "pie"] as const;
export type ChartKind = (typeof CHART_KINDS)[number];

export function isChartKind(kind: string): kind is ChartKind {
  return (CHART_KINDS as readonly string[]).includes(kind);
}

export type DisplayKindView = { label: string; icon: LucideIcon };

const KIND_VIEW: Record<string, DisplayKindView | undefined> = {
  table: { label: "Table", icon: Table2 },
  markdown: { label: "Text", icon: FileText },
  bar: { label: "Bar chart", icon: BarChart3 },
  line: { label: "Line chart", icon: LineChart },
  area: { label: "Area chart", icon: AreaChart },
  scatter: { label: "Scatter chart", icon: ScatterChart },
  pie: { label: "Pie chart", icon: PieChart },
};

const GENERIC_VIEW: DisplayKindView = { label: "Display", icon: Shapes };

export function displayKindView(kind: string): DisplayKindView {
  return KIND_VIEW[kind] ?? GENERIC_VIEW;
}

/** The display's own title, else its kind ("Table", "Bar chart"). */
export function displayTitle(display: PythonDisplay): string {
  const title = display.title?.replace(/\s+/g, " ").trim();
  return title ? title : displayKindView(display.kind).label;
}

export function displayRowsTotal(display: PythonDisplay): number {
  const shown = display.rows?.length ?? 0;
  return Math.max(display.rows_total ?? shown, shown);
}

const numberFormat = new Intl.NumberFormat("en-US");

export function formatCount(value: number): string {
  return numberFormat.format(value);
}

function counted(value: number, one: string, many = `${one}s`) {
  return `${formatCount(value)} ${value === 1 ? one : many}`;
}

/** "120 rows × 4 columns", "40 points", "1,204 characters". */
export function displaySize(display: PythonDisplay): string {
  if (display.kind === "markdown")
    return counted(Array.from(display.text ?? "").length, "character");
  const rows = displayRowsTotal(display);
  const columns = display.columns?.length ?? 0;
  if (isChartKind(display.kind)) return counted(rows, "point");
  if (display.kind === "table")
    return `${counted(rows, "row")} × ${counted(columns, "column")}`;
  return "";
}

/** "Showing 200 of 12,000 rows" when the inline rows were cut. */
export function rowsNote(display: PythonDisplay): string | undefined {
  const shown = display.rows?.length ?? 0;
  const total = displayRowsTotal(display);
  if (!display.truncated && total <= shown) return undefined;
  if (total > shown)
    return `Showing ${formatCount(shown)} of ${formatCount(total)} rows`;
  return "Showing part of the data";
}

export function cellText(cell: PythonDisplayCell | undefined): string {
  if (cell === null || cell === undefined) return "";
  return String(cell);
}

/** Columns whose non-null cells are all numbers. */
export function numericColumns(
  columns: readonly string[],
  rows: readonly PythonDisplayCell[][],
): boolean[] {
  return columns.map((_, index) => {
    let any = false;
    for (const row of rows) {
      const cell = row.at(index) ?? null;
      if (cell === null) continue;
      if (typeof cell !== "number") return false;
      any = true;
    }
    return any;
  });
}

function tsvField(text: string): string {
  return text.replace(/[\t\r\n]+/g, " ");
}

/** Tab-separated text with a header row, for Copy. */
export function displayTsv(display: PythonDisplay): string {
  const columns = display.columns ?? [];
  const rows = display.rows ?? [];
  const width = Math.max(columns.length, ...rows.map((row) => row.length), 0);
  const header = Array.from({ length: width }, (_, index) =>
    tsvField(columns[index] ?? ""),
  );
  const lines = [header.join("\t")];
  for (const row of rows)
    lines.push(
      Array.from({ length: width }, (_, index) =>
        tsvField(cellText(row[index])),
      ).join("\t"),
    );
  return lines.join("\n");
}

/** Text Copy puts on the clipboard: TSV for data kinds, the text for markdown. */
export function displayCopyText(display: PythonDisplay): string {
  if (display.kind === "markdown") return display.text ?? "";
  return displayTsv(display);
}

export type ChartSeries = {
  name: string;
  /** One value per category; null where the row's value is not a number. */
  values: (number | null)[];
};

export type ChartData = {
  kind: ChartKind;
  categories: string[];
  series: ChartSeries[];
  /** Points that carry a value in at least one series. */
  points: number;
};

function columnIndex(columns: readonly string[], name: string | undefined) {
  return name === undefined ? -1 : columns.indexOf(name);
}

/**
 * Chart data from `columns`/`rows` with `x` and `y`: x labels the category
 * (or the scatter axis), each y column is a series; a row whose y value is
 * not a number is skipped for that series. Without `x` the row number labels
 * the point; without `y` every other numeric column is a series.
 */
export function chartData(display: PythonDisplay): ChartData | undefined {
  if (!isChartKind(display.kind)) return undefined;
  const columns = display.columns ?? [];
  const rows = display.rows ?? [];
  const xIndex = columnIndex(columns, display.x);
  const numeric = numericColumns(columns, rows);
  const yNames =
    display.y && display.y.length > 0
      ? display.y.filter((name) => columns.includes(name))
      : columns.filter((_, index) => index !== xIndex && numeric[index]);
  const series = (display.kind === "pie" ? yNames.slice(0, 1) : yNames).map(
    (name) => {
      const index = columns.indexOf(name);
      return {
        name,
        values: rows.map((row) => {
          const cell = row[index];
          return typeof cell === "number" && Number.isFinite(cell)
            ? cell
            : null;
        }),
      };
    },
  );
  if (series.length === 0) return undefined;
  const categories = rows.map((row, index) =>
    xIndex >= 0 ? cellText(row[xIndex]) : String(index + 1),
  );
  const points = rows.filter((_, index) =>
    series.some((item) => item.values[index] !== null),
  ).length;
  return { kind: display.kind, categories, series, points };
}

/** Generated accessible name of a chart: kind, title, points, series. */
export function chartLabel(display: PythonDisplay, data: ChartData): string {
  const kind = displayKindView(display.kind).label;
  const title = display.title?.trim();
  const names = data.series.map((item) => item.name).join(", ");
  return [
    title ? `${kind}: ${title}` : kind,
    counted(data.points, "point"),
    `series ${names}`,
  ].join(", ");
}

/** "41.2k", "1.3M", "950". */
export function compactCount(value: number): string {
  const format = (n: number, unit: string) =>
    `${n >= 100 ? Math.round(n) : Number(n.toFixed(1))}${unit}`;
  if (value >= 1_000_000) return format(value / 1_000_000, "M");
  if (value >= 1_000) return format(value / 1_000, "k");
  return String(value);
}

export const MODEL_TOOL = "llm";
/** Script helpers that are highlighted like tool names. */
export const HELPER_CALLEES = ["llm", "show", "engine"] as const;

export function isModelCall(call: PythonCallRecord): boolean {
  return call.tool === MODEL_TOOL;
}

export function callTokens(call: PythonCallRecord): number | undefined {
  if (call.tokens_in === undefined && call.tokens_out === undefined)
    return undefined;
  return (call.tokens_in ?? 0) + (call.tokens_out ?? 0);
}

/** "41.2k tokens" for a set of model calls, or undefined without counts. */
export function callsTokens(
  calls: readonly PythonCallRecord[],
): string | undefined {
  let total = 0;
  let any = false;
  for (const call of calls) {
    const tokens = callTokens(call);
    if (tokens === undefined) continue;
    any = true;
    total += tokens;
  }
  return any ? `${compactCount(total)} tokens` : undefined;
}

export type LlmPreview = {
  prompt?: string;
  model?: string;
  schema: boolean;
};

/** The `{prompt, model?, schema}` preview of a model call. */
export function readLlmPreview(argsPreview: string): LlmPreview {
  try {
    const value: unknown = JSON.parse(argsPreview);
    if (typeof value === "object" && value !== null && !Array.isArray(value)) {
      const record = value as Record<string, unknown>;
      return {
        prompt: typeof record.prompt === "string" ? record.prompt : undefined,
        model:
          typeof record.model === "string" && record.model.trim()
            ? record.model
            : undefined,
        schema: record.schema === true,
      };
    }
  } catch {
    /* fall through to the raw text */
  }
  return { prompt: argsPreview.trim() || undefined, schema: false };
}

/** Model of a call: the record's own, else the one the script asked for. */
export function callModel(call: PythonCallRecord): string | undefined {
  const own = call.model?.trim();
  return own ? own : readLlmPreview(call.args_preview).model;
}

/** "in 12.4k · out 820" */
export function tokensInOut(call: PythonCallRecord): string | undefined {
  if (call.tokens_in === undefined && call.tokens_out === undefined)
    return undefined;
  return `in ${compactCount(call.tokens_in ?? 0)} · out ${compactCount(
    call.tokens_out ?? 0,
  )}`;
}

/** Card meta counts: "2 calls · 12 model calls" with both kinds told apart. */
export function callCounts(
  calls: readonly PythonCallRecord[],
  total: number,
  modelTotal?: number,
): { tools: number; models: number } {
  const seenModels = calls.filter(isModelCall).length;
  const models = Math.max(modelTotal ?? seenModels, seenModels);
  return { tools: Math.max(0, total - models), models };
}

/**
 * A display's data file may only be opened when it lies inside the folder
 * the run or the workspace view reported (after normalising `..` segments).
 */
export function isDisplayPathInside(
  path: string,
  baseDir: string | undefined,
): boolean {
  if (!baseDir || !path) return false;
  return isInsideDir(path, baseDir);
}

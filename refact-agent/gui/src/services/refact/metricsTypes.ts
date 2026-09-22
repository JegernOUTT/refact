export const metricSampleKinds = ["counter", "gauge", "observation"] as const;
export type MetricSampleKind = (typeof metricSampleKinds)[number];

export type MetricSample =
  | { kind: "counter"; value: number }
  | { kind: "gauge"; value: number }
  | {
      kind: "observation";
      count: number;
      sum: number;
      min: number;
      max: number;
    };

export type MetricLabel = { name: string; value: string };

export type MetricSeries = {
  name: string;
  labels: MetricLabel[];
  sample: MetricSample;
};

export const metricDecisions = [
  "automatic_pause",
  "denial",
  "retry",
  "merge_refusal",
  "backpressure_deferral",
  "goal_completion",
  "goal_refusal",
] as const;
export type MetricDecision = (typeof metricDecisions)[number];

export type MetricExplanation = {
  decision: MetricDecision;
  outcome: string;
  rule: string;
  inputs: [string, string][];
  seq: number;
  at_ms: number;
};

export type MetricEntry =
  | { kind: "series"; series: MetricSeries }
  | { kind: "explanation"; explanation: MetricExplanation };

export type MetricsPage = {
  snapshot_id: string;
  seq: number;
  project: string;
  entries: MetricEntry[];
  total: number;
  series_total: number;
  explanations_total: number;
  returned: number;
  truncated: boolean;
  next_cursor?: string;
  generated_at_ms: number;
};

const object = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const integer = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const number = (value: unknown): value is number =>
  typeof value === "number" && Number.isFinite(value);

function isSample(value: unknown): value is MetricSample {
  if (
    !object(value) ||
    !metricSampleKinds.includes(value.kind as MetricSampleKind)
  )
    return false;
  if (value.kind === "counter" || value.kind === "gauge")
    return number(value.value);
  return (
    integer(value.count) &&
    number(value.sum) &&
    number(value.min) &&
    number(value.max)
  );
}

function isSeries(value: unknown): value is MetricSeries {
  return (
    object(value) &&
    typeof value.name === "string" &&
    Array.isArray(value.labels) &&
    value.labels.every(
      (label) =>
        object(label) &&
        typeof label.name === "string" &&
        typeof label.value === "string",
    ) &&
    isSample(value.sample)
  );
}

function isExplanation(value: unknown): value is MetricExplanation {
  return (
    object(value) &&
    metricDecisions.includes(value.decision as MetricDecision) &&
    typeof value.outcome === "string" &&
    typeof value.rule === "string" &&
    Array.isArray(value.inputs) &&
    value.inputs.every(
      (input) =>
        Array.isArray(input) &&
        input.length === 2 &&
        typeof input[0] === "string" &&
        typeof input[1] === "string",
    ) &&
    integer(value.seq) &&
    integer(value.at_ms)
  );
}

export function isMetricEntry(value: unknown): value is MetricEntry {
  if (!object(value)) return false;
  if (value.kind === "series") return isSeries(value.series);
  if (value.kind === "explanation") return isExplanation(value.explanation);
  return false;
}

export function parseMetricsPage(value: unknown): MetricsPage | null {
  if (
    !object(value) ||
    typeof value.snapshot_id !== "string" ||
    !integer(value.seq) ||
    typeof value.project !== "string" ||
    !Array.isArray(value.entries) ||
    !value.entries.every(isMetricEntry) ||
    !integer(value.total) ||
    !integer(value.series_total) ||
    !integer(value.explanations_total) ||
    !integer(value.returned) ||
    typeof value.truncated !== "boolean" ||
    (value.next_cursor !== undefined &&
      typeof value.next_cursor !== "string") ||
    !integer(value.generated_at_ms)
  )
    return null;
  if (value.entries.length !== value.returned || value.returned > value.total)
    return null;
  const explanations = value.entries.filter(
    (entry): entry is Extract<MetricEntry, { kind: "explanation" }> =>
      entry.kind === "explanation",
  );
  if (
    explanations.some(
      ({ explanation }) =>
        explanation.rule.trim().length === 0 ||
        explanation.outcome.trim().length === 0 ||
        explanation.inputs.length === 0,
    )
  )
    return null;
  return value as MetricsPage;
}

export function seriesLabel(series: MetricSeries): string {
  if (series.labels.length === 0) return series.name;
  return `${series.name}{${series.labels
    .map(({ name, value }) => `${name}=${value}`)
    .join(",")}}`;
}

export function sampleLabel(sample: MetricSample): string {
  if (sample.kind === "counter" || sample.kind === "gauge")
    return String(sample.value);
  return `${sample.count} observations · sum ${sample.sum} · ${sample.min}–${sample.max}`;
}

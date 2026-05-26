export const DOCUMENT_KINDS = [
  "plan",
  "design",
  "runbook",
  "brief",
  "postmortem",
  "spec",
] as const;
export type KnownDocumentKind = (typeof DOCUMENT_KINDS)[number];

export const MEMORY_KINDS = [
  "decision",
  "spec",
  "finding",
  "gotcha",
  "risk",
  "handoff",
  "progress",
  "postmortem",
  "brief",
  "freeform",
] as const;
export type KnownMemoryKind = (typeof MEMORY_KINDS)[number];

type BadgeColor =
  | "blue"
  | "purple"
  | "green"
  | "teal"
  | "red"
  | "gray"
  | "amber";

const DOCUMENT_KIND_COLORS: Record<KnownDocumentKind, BadgeColor> = {
  plan: "blue",
  design: "purple",
  runbook: "green",
  brief: "teal",
  postmortem: "red",
  spec: "gray",
};

const MEMORY_KIND_COLORS: Record<KnownMemoryKind, BadgeColor> = {
  decision: "purple",
  spec: "blue",
  finding: "green",
  gotcha: "amber",
  risk: "red",
  handoff: "purple",
  progress: "blue",
  postmortem: "amber",
  brief: "green",
  freeform: "gray",
};

export function documentKindColor(kind: string): BadgeColor {
  return DOCUMENT_KIND_COLORS[kind as KnownDocumentKind] ?? "gray";
}

export function memoryKindColor(kind: string): BadgeColor {
  return MEMORY_KIND_COLORS[kind as KnownMemoryKind] ?? "gray";
}

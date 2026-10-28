export type TranscriptSectionId =
  | "summary"
  | "input"
  | "output"
  | "artifacts"
  | "error"
  | "actions"
  | "raw";

export const TRANSCRIPT_SECTION_ORDER: readonly TranscriptSectionId[] = [
  "summary",
  "input",
  "output",
  "artifacts",
  "error",
  "actions",
  "raw",
];

export const DEFAULT_SECTION_LABELS: Record<TranscriptSectionId, string> = {
  summary: "Summary",
  input: "Input",
  output: "Output",
  artifacts: "Artifacts",
  error: "Error",
  actions: "Actions",
  raw: "Raw",
};

export const evidenceVerdicts = [
  "not_run",
  "passed",
  "failed",
  "blocked",
  "unavailable",
  "superseded",
] as const;
export type EvidenceVerdict = (typeof evidenceVerdicts)[number];

const object = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

export function parseEvidenceVerdict(value: unknown): EvidenceVerdict | null {
  return evidenceVerdicts.find((verdict) => verdict === value) ?? null;
}

export function goalEvidenceVerdict(extra: unknown): EvidenceVerdict | null {
  if (!object(extra)) return null;
  const receipt = extra.validate_goal;
  if (!object(receipt)) return null;
  return parseEvidenceVerdict(receipt.evidence_verdict);
}

import type { CausalEvent, CausalPage } from "./causalTypes";

export const causalEventFixture: CausalEvent = {
  event_id: "decision:d",
  seq: 4,
  kind: "decision",
  scope: "parent",
  work: { runtime: "trajectory", id: "parent" },
  causes: [{ event: "evidence:f" }],
  recorded_at_ms: 1_004,
};

export const causalPageFixture: CausalPage = {
  snapshot_id: "snapshot",
  seq: 1,
  project: "project",
  root: "decision:d",
  events: [
    causalEventFixture,
    {
      event_id: "evidence:f",
      seq: 3,
      kind: "evidence",
      scope: "child",
      work: { runtime: "background_agent", id: "child" },
      causes: ["unknown"],
      recorded_at_ms: 1_003,
    },
  ],
  total: 2,
  returned: 2,
  truncated: false,
  complete: false,
  gaps: ["evidence:f:unknown"],
  gap_count: 1,
  durable: true,
  generated_at_ms: 1_005,
  replay: {
    root: "decision:d",
    steps: 2,
    findings: [],
    finding_count: 0,
    reproduces_model_output: false,
    scope_note:
      "Replay re-runs the recorded coordination decisions only: admission order, resolved causes and the completeness verdict. It does not re-run any model and does not reproduce model output.",
  },
};

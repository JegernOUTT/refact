import type { WorkPage } from "./workTypes";

export const workFixture: WorkPage = {
  snapshot_id: "snapshot",
  seq: 1,
  project: "project",
  coverage: "partial",
  page_unit: "work",
  rows: [
    {
      key: { runtime: "trajectory", id: "chat" },
      kind: "board_agent",
      state: "queued",
      attention: false,
      title: "Board work",
      root: {
        id: "root",
        kind: "project",
        project: "project",
        path: "/project",
      },
      freshness: "live",
      declared_files: [],
      observed_files: [],
      observed_reads: [],
      updated_at_ms: 1,
      measured_tokens: null,
      measured_cost_cents: null,
    },
  ],
  roots: [],
  total: 1,
  returned: 1,
  truncated: false,
  generated_at_ms: 1,
};

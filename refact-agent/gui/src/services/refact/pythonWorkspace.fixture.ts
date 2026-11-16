export const workspaceExample = {
  schema: 1,
  chat_id: "chat-a",
  exists: true,
  workspace_dir: "/python",
  artifacts_dir: "/python/artifacts",
  session: {
    status: "idle",
    live: false,
    state_persisted: true,
    state_bytes: 1024,
    saved_at_ms: 1700000000000,
    last_run_id: null,
    reset_reason: null,
    variables: [{ name: "answer", type_name: "int", preview: "42" }],
    variables_truncated: false,
  },
  artifacts: [
    {
      path: "/python/artifacts/chart.png",
      rel_path: "chart.png",
      bytes: 1024,
      modified_at_ms: 1700000000000,
    },
  ],
  artifacts_truncated: false,
  runs: [
    {
      run_id: "run-one",
      status: "finished",
      started_at_ms: 1700000000000,
      finished_at_ms: 1700000001000,
      calls_total: 2,
      background: true,
    },
  ],
  runs_truncated: false,
  disk: { workspace_bytes: 2048, quota_bytes: 4096 },
};

export const pythonCall = {
  seq: 1,
  call_id: "c",
  tool: "shell",
  args_preview: "ls",
  status: "running" as const,
  summary: "",
  started_at_ms: 1,
  duration_ms: null,
  approval: "not_needed" as const,
  diffs: [],
};

export const pythonApproval = {
  request_id: "q",
  tool_call_id: "t",
  run_id: "r",
  call_id: "c",
  tool_name: "shell",
  args_preview: "ls",
  command: null,
  rule: "confirm",
  editing_tool: false,
  created_at_ms: 1,
};

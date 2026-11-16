import type {
  PythonApprovalRequest,
  PythonCallRecord,
  PythonEngineChange,
  PythonEngineRequest,
} from "../../../services/refact/types";

export const engineRequestBase: PythonApprovalRequest = {
  request_id: "e1",
  tool_call_id: "t",
  run_id: "r",
  call_id: "c",
  tool_name: "engine.settings.set",
  args_preview: '{"key":"autonomy.max_steps","value":40}',
  command: null,
  rule: "engine",
  editing_tool: false,
  created_at_ms: 1,
};

export function engineRequest(
  engine: Partial<PythonEngineRequest>,
  extra: Partial<PythonApprovalRequest> = {},
): PythonApprovalRequest {
  return {
    ...engineRequestBase,
    ...extra,
    engine: {
      function: "settings.set",
      class: "change",
      title: "change engine settings",
      items: [{ label: "autonomy.max_steps", old: 25, new: 40 }],
      scope: "project",
      ...engine,
    },
  };
}

export const engineChange = (
  seq: number,
  change: Partial<PythonEngineChange> = {},
): PythonEngineChange => ({
  seq,
  function: "settings.set",
  class: "change",
  title: "change engine settings",
  items: [{ label: "autonomy.max_steps", old: 25, new: 40 }],
  scope: "project",
  at_ms: 1000 + seq,
  ...change,
});

/** A change the engine started but could not confirm (a timeout). */
export const unknownOutcomeChange = (seq: number): PythonEngineChange =>
  engineChange(seq, { outcome: "unknown" });

/** A refusal of about 300 characters, the longest the engine sends for a call. */
export const ENGINE_REFUSAL_MESSAGE =
  "The engine refused to change autonomy.max_steps because the project settings are locked by the workspace policy and only the owner of the workspace can unlock them from the settings page. " +
  "Ask the owner to unlock the project settings, or change this value for the current chat only instead.";

export const engineCall = (
  seq: number,
  tool: string,
  line: number,
  record: Partial<PythonCallRecord> = {},
): PythonCallRecord => ({
  seq,
  call_id: `e${seq}`,
  tool,
  args_preview: '{"key":"autonomy.max_steps"}',
  status: "ok",
  summary: "",
  started_at_ms: seq * 100,
  duration_ms: 12,
  approval: "not_needed",
  diffs: [],
  line,
  ...record,
});

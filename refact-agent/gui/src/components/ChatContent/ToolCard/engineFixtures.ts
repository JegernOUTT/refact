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

/** One unbroken 400-character line inside an edited message. */
export const PATCH_LONG_LINE = `Logs:${"x".repeat(395)}`;

/**
 * A `chats.patch` request (trajectory control): an edit whose old and new
 * texts span several lines, one of them long, a removed message and an
 * inserted one, with the engine's warning.
 */
export const chatsPatchRequest = engineRequest(
  {
    function: "chats.patch",
    class: "destructive",
    title: "change messages in another chat",
    scope: "chat Parser refactor (6b1d0c2e-8f0a-4d55-9a57-1f2e3d4c5b6a)",
    warning:
      "The original messages will be overwritten and cannot be restored.",
    items: [
      {
        label: "message 2 (user)",
        old: [
          "Refactor the parser so every token keeps its source span.",
          "Keep the public API unchanged.",
          "Run the whole test suite before you report.",
          PATCH_LONG_LINE,
          "Report every failing case.",
        ].join("\n"),
        new: [
          "Refactor the parser so every token keeps its source span.",
          "Keep the public API unchanged; deprecate nothing.",
          "Run the whole test suite before you report.",
          PATCH_LONG_LINE,
          "Report every failing case with its input.",
        ].join("\n"),
      },
      {
        label: "message 4 (assistant)",
        old: Array.from(
          { length: 12 },
          (_, index) => `Step ${index + 1}: updated the lexer tables.`,
        ).join("\n"),
      },
      {
        label: "new message after 4 (user)",
        new: [
          "Now add a regression test for spans.",
          "Use the fixtures under tests/parser.",
          "Do not touch the CLI.",
        ].join("\n"),
      },
    ],
  },
  {
    request_id: "patch",
    tool_name: "engine.chats.patch",
    args_preview: '{"chat_id":"6b1d0c2e","ops":"[3 operations]"}',
  },
);

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

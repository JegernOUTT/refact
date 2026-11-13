import type { ToolCall, ToolMessage } from "../../../services/refact/types";

export const launchPreviewCall: ToolCall = {
  id: "launch-preview",
  index: 0,
  type: "function",
  function: {
    name: "subagent",
    arguments: JSON.stringify({
      action: "preview",
      task: "Inspect accepted context",
      expected_result: "An evidence-backed outcome",
      launch_spec: { history_mode: "fork_completed_prefix" },
    }),
  },
};

export function launchPreviewReceipt(): ToolMessage {
  return {
    role: "tool",
    tool_call_id: launchPreviewCall.id ?? "launch-preview",
    content: "# Launch Preview (no child spawned)",
    extra: {
      preview: true,
      estimated_tokens: 840,
      serialized_bytes: 3360,
      skipped_count: 1,
      effective_model: "local/qualified-model",
      effective_tools: ["cat", "mcp_fixture_read"],
      deliverable: {
        required_sections: ["Findings", "Evidence"],
        output_schema: { type: "object" },
        evidence_required: true,
      },
      context_sources: [
        {
          type: "file_range",
          path: "src/accepted-context.rs",
          line1: 2,
          line2: 5,
          digest: "accepted-source-digest",
          observed_at_ms: 1789233578382,
        },
        {
          type: "completed_prefix",
          principal: "parent-chat",
          message_id: "completed-parent-message",
          digest: "completed-message-digest",
          observed_at_ms: 1789233578382,
        },
      ],
    },
  };
}

// Exact envelope emitted before effective launch settings are available.
export function blockedLaunchPreviewReceipt(): ToolMessage {
  return {
    role: "tool",
    tool_call_id: launchPreviewCall.id ?? "launch-preview",
    content:
      "# Launch Preview (no child spawned)\n\n## Launch Blocked\n- launch denied",
    extra: {
      preview: true,
      skipped_count: 0,
      estimated_tokens: 0,
      serialized_bytes: 0,
      context_sources: [],
      skipped: [],
      exclusions: [],
      launch_blocked: "launch denied",
    },
  };
}

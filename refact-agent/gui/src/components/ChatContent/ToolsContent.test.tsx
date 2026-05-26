import { describe, expect, it } from "vitest";
import { render, screen, createDefaultChatState } from "../../utils/test-utils";
import type {
  ChatMessages,
  ToolCall,
  ToolMessage,
} from "../../services/refact/types";
import { ToolContent } from "./ToolsContent";

const CHECK_AGENTS_OUTPUT = `⚠️  Alerts: 1 stuck (>15min), 0 failed, 0 needing approval

P0 🔄  T-1   implement-render       | generating |  3m ago | last: cat
P1 🔴  T-2   fix-tests              | STUCK 18m   | needs attention
showing 2 of 2; no more pages
`;

const AGENT_PULSE_OUTPUT = `# Agent Pulse: T-29

**Card:** check-agents redesign
**State:** 🔄 generating response
**Last activity:** 3m ago
**Tokens used:** ~38k / 200k
**Currently editing:** src/tools/tool_check_agents.rs

## Last assistant message
> Adding sticky alerts logic...

## Last tool call
\`patch(path="src/tools/tool_check_agents.rs")\`
`;

const AGENT_DIFF_OUTPUT = [
  "# Agent Diff for T-29",
  "",
  "**Card:** check-agents redesign",
  "**Branch:** refact/task/T-29-agent",
  "**Base:** commit abc123",
  "",
  "```diff",
  "diff --git a/src/file.ts b/src/file.ts",
  "index 1111111..2222222 100644",
  "--- a/src/file.ts",
  "+++ b/src/file.ts",
  "@@ -1 +1 @@",
  "-old line",
  "+new line",
  "```",
].join("\n");

const DOC_LIST_OUTPUT = [
  "| slug | name | kind | pinned | version | updated_at |",
  "|---|---|---|---|---:|---|",
  "| main-plan | Main Plan | plan | true | 3 | 2026-05-22T10:00:00Z |",
].join("\n");

const DOC_GET_OUTPUT = [
  "---",
  "name: Main Plan",
  "slug: main-plan",
  "kind: plan",
  "pinned: true",
  "version: 3",
  "---",
  "",
  "# Main Plan",
  "",
  "- Ship document renderer",
].join("\n");

function structuredFinalReport(success: boolean) {
  return JSON.stringify({
    summary: "Added routing tests.",
    success,
    files_changed: ["src/components/ChatContent/ToolsContent.test.tsx"],
    tests_added_or_updated: ["ToolsContent.test.tsx"],
    verification: [
      {
        command: "npm run test -- ToolsContent --run",
        exit_code: 0,
        passed: true,
        output_tail: "passed",
      },
    ],
    followup_cards: [],
    risks: [],
    assumptions: [],
  });
}

const STRUCTURED_FINAL_REPORT = structuredFinalReport(true);

const TASK_DONE_OUTPUT = JSON.stringify({
  type: "task_done",
  summary: "Task completed",
  report: "Done",
  files_changed: ["src/file.ts"],
});

function makeToolCall(name: string, id: string): ToolCall {
  return {
    id,
    index: 0,
    type: "function",
    function: {
      name,
      arguments: "{}",
    },
  };
}

function makeToolMessage(id: string, content: string): ToolMessage {
  return {
    role: "tool",
    tool_call_id: id,
    content,
    tool_failed: false,
  };
}

function renderToolContent(name: string, content: string) {
  const id = `call-${name.replace(/[^a-z0-9]+/gi, "-")}`;
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
  if (!runtime) throw new Error("missing test thread");
  runtime.thread.messages = [makeToolMessage(id, content)] as ChatMessages;

  return render(<ToolContent toolCalls={[makeToolCall(name, id)]} />, {
    preloadedState: { chat },
  });
}

describe("ToolsContent routing", () => {
  it.each([
    ["check_agents", CHECK_AGENTS_OUTPUT, "agent-status-view"],
    ["agent_pulse", AGENT_PULSE_OUTPUT, "agent-pulse-view"],
    ["agent_diff", AGENT_DIFF_OUTPUT, "agent-diff-view"],
    ["doc_list", DOC_LIST_OUTPUT, "task-documents-view"],
    ["doc_get", DOC_GET_OUTPUT, "task-documents-view"],
    ["agent_finish", STRUCTURED_FINAL_REPORT, "final-report-tool"],
    ["task_done", TASK_DONE_OUTPUT, "task-done-tool"],
    ["unknown_tool", "unknown result", "generic-tool"],
  ])("routes %s to %s", (name, content, testId) => {
    renderToolContent(name, content);

    expect(screen.getByTestId(testId)).toBeInTheDocument();
  });

  it("routes plain-text agent_finish results through FinalReportView legacy fallback", () => {
    renderToolContent("agent_finish", "Plain legacy report");

    expect(screen.queryByTestId("generic-tool")).not.toBeInTheDocument();
    expect(screen.queryByTestId("final-report-view")).not.toBeInTheDocument();
    expect(screen.getByTestId("final-report-tool")).toBeInTheDocument();
    expect(screen.getByText("Plain legacy report")).toBeInTheDocument();
  });

  it("final_report_tool_card_uses_hidden_marker_not_display_contents", () => {
    const { container } = renderToolContent(
      "agent_finish",
      STRUCTURED_FINAL_REPORT,
    );

    expect(screen.getByTestId("final-report-tool")).toHaveAttribute("hidden");
    expect(
      Array.from(container.querySelectorAll<HTMLElement>("[style]")).some(
        (element) => element.style.display === "contents",
      ),
    ).toBe(false);
  });

  it("final_report_tool_card_shows_error_when_success_false", () => {
    renderToolContent("agent_finish", structuredFinalReport(false));

    expect(
      screen.getByTestId("final-report-tool-error-icon"),
    ).toBeInTheDocument();
    expect(
      screen.queryByTestId("final-report-tool-success-icon"),
    ).not.toBeInTheDocument();
    expect(screen.getAllByText("failed")).not.toHaveLength(0);
  });

  it("final_report_tool_card_shows_success_when_success_true", () => {
    renderToolContent("agent_finish", structuredFinalReport(true));

    expect(
      screen.getByTestId("final-report-tool-success-icon"),
    ).toBeInTheDocument();
    expect(
      screen.queryByTestId("final-report-tool-error-icon"),
    ).not.toBeInTheDocument();
    expect(screen.getByText("success")).toBeInTheDocument();
  });
});

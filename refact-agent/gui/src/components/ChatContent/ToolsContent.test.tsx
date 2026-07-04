import { describe, expect, it } from "vitest";
import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
} from "../../utils/test-utils";
import type {
  ChatMessages,
  ExecToolMetadata,
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

function makeToolCall(
  name: string,
  id: string,
  args: Record<string, unknown> = {},
): ToolCall {
  return {
    id,
    index: 0,
    type: "function",
    function: {
      name,
      arguments: JSON.stringify(args),
    },
  };
}

function makeToolMessage(
  id: string,
  content: string,
  extra?: ExecToolMetadata,
): ToolMessage {
  return {
    role: "tool",
    tool_call_id: id,
    content,
    tool_failed: false,
    extra: extra ? { exec: extra } : undefined,
  };
}

function renderToolContent(
  name: string,
  content: string,
  options: { args?: Record<string, unknown>; extra?: ExecToolMetadata } = {},
) {
  const id = `call-${name.replace(/[^a-z0-9]+/gi, "-")}`;
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
  if (!runtime) throw new Error("missing test thread");
  runtime.thread.messages = [
    makeToolMessage(id, content, options.extra),
  ] as ChatMessages;

  return render(
    <ToolContent toolCalls={[makeToolCall(name, id, options.args)]} />,
    {
      preloadedState: { chat },
    },
  );
}

function openToolCard() {
  fireEvent.click(screen.getByRole("button", { expanded: false }));
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
    ["process_start", "Process started", "exec-tool-process_start"],
    ["process_list", "Processes", "exec-tool-process_list"],
    ["process_read", "Process output", "exec-tool-process_read"],
    ["process_kill", "Process killed", "exec-tool-process_kill"],
    ["process_wait", "Process wait completed", "exec-tool-process_wait"],
    ["unknown_tool", "unknown result", "generic-tool"],
  ])("routes %s to %s", (name, content, testId) => {
    renderToolContent(name, content);

    expect(screen.getByTestId(testId)).toBeInTheDocument();
  });

  it.each([
    "code_health",
    "git_risk",
    "code_why",
    "codegraph_overview",
    "code_duplication",
    "code_map",
    "security_scan",
    "pr_blast",
    "dead_code",
  ])("routes %s to EngineAnalysisTool", (name) => {
    renderToolContent(name, "# Analysis\n\n- useful result");

    expect(screen.getByTestId("engine-analysis-tool")).toBeInTheDocument();
    expect(screen.queryByTestId("generic-tool")).not.toBeInTheDocument();
  });

  it("renders security_scan summary instead of GenericTool fallback", () => {
    renderToolContent(
      "security_scan",
      [
        "Security scan for `src/server.ts` found 2 findings (lang: TypeScript).",
        "Severity counts: Critical=1 High=0 Medium=1 Low=0",
        "",
        "  src/server.ts:12 [Critical] dangerous-eval — eval(userInput)",
        "  src/server.ts:20 [Medium] insecure-random — Math.random()",
      ].join("\n"),
    );

    expect(screen.getByTestId("engine-analysis-tool")).toBeInTheDocument();
    expect(screen.queryByTestId("generic-tool")).not.toBeInTheDocument();
    openToolCard();
    expect(screen.getAllByText("dangerous-eval").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Critical 1").length).toBeGreaterThan(0);
    expect(screen.getAllByText("eval(userInput)").length).toBeGreaterThan(0);
  });

  it("renders pr_blast summary with reviewers and impact kinds", () => {
    renderToolContent(
      "pr_blast",
      [
        "⚠ index still building (3 files queued, 5 cross-file edges) — impact may be under-reported",
        "PR blast radius (max depth 3) for 2 changed files:",
        "Index state: queued=3 cross_file_edges=5 cross_file_ready=false partial=true",
        "  changed: src/main.rs",
        "  changed: src/router.ts",
        "",
        "Impacted files: 2",
        "Risk score: 0.62",
        "",
        "Directly impacted symbols (1):",
        "  d1 renderApp @ src/app.rs via calls (behavioral)",
        "",
        "Transitively impacted symbols (1):",
        "  d2 createStore @ src/state.ts via inherits (structural)",
        "",
        "Suggested reviewers (git ownership):",
        "  ada@example.com (score 0.95)",
      ].join("\n"),
    );

    expect(screen.getByTestId("engine-analysis-tool")).toBeInTheDocument();
    expect(screen.queryByTestId("generic-tool")).not.toBeInTheDocument();
    openToolCard();
    expect(screen.getAllByText("partial index").length).toBeGreaterThan(0);
    expect(screen.getAllByText("structural").length).toBeGreaterThan(0);
    expect(screen.getAllByText(/ada@example.com/).length).toBeGreaterThan(0);
  });

  it("renders dead_code confidence preview", () => {
    renderToolContent(
      "dead_code",
      [
        "Dead code candidates: 1 shown of 4 matching candidates.",
        "Index state: queued=0 dirty_paths=0 pending_refs=0 cross_file_edges=10 cross_file_ready=true partial=false",
        "",
        "src/unused.ts:",
        "  0.91  line 42  unusedHelper — unreachable; last touched 400d ago; churn 1 in mined window",
      ].join("\n"),
    );

    expect(screen.getByTestId("engine-analysis-tool")).toBeInTheDocument();
    expect(screen.queryByTestId("generic-tool")).not.toBeInTheDocument();
    openToolCard();
    expect(screen.getAllByText("unusedHelper").length).toBeGreaterThan(0);
    expect(screen.getAllByText("confidence 0.9100").length).toBeGreaterThan(0);
  });

  it("routes plain-text agent_finish results through FinalReportView legacy fallback", () => {
    renderToolContent("agent_finish", "Plain legacy report");

    expect(screen.queryByTestId("generic-tool")).not.toBeInTheDocument();
    expect(screen.queryByTestId("final-report-view")).not.toBeInTheDocument();
    expect(screen.getByTestId("final-report-tool")).toBeInTheDocument();
    expect(screen.getByText("Plain legacy report")).toBeInTheDocument();
  });

  it("routes service-mode integration output with exec metadata to ExecToolCard", () => {
    renderToolContent(
      "service_dev_server",
      "stdout:\nready\nstderr:\n<empty>\n",
      {
        args: { command: "npm run dev" },
        extra: {
          process_id: "exec_service_dev",
          status: "running",
          short_description: "Dev server",
          command: "npm run dev",
          mode: "service",
          cwd: "/workspace",
        },
      },
    );

    expect(screen.queryByTestId("generic-tool")).not.toBeInTheDocument();
    expect(screen.getByTestId("exec-tool-exec")).toBeInTheDocument();
    expect(screen.getByText("Dev server")).toBeInTheDocument();
    expect(screen.getByText("exec_service_dev")).toBeInTheDocument();
  });

  it("renders tool results from an explicit non-current thread", () => {
    const id = "call-agent-finish-thread-b";
    const chat = createDefaultChatState();
    const currentId = chat.current_thread_id;
    const currentRuntime = chat.threads[currentId];
    const otherRuntime = {
      ...currentRuntime,
      thread: {
        ...currentRuntime.thread,
        id: "thread-b",
        messages: [makeToolMessage(id, "Right thread report")] as ChatMessages,
      },
    };
    currentRuntime.thread.messages = [
      makeToolMessage(id, "Wrong current report"),
    ] as ChatMessages;
    chat.threads["thread-b"] = otherRuntime;
    chat.open_thread_ids = [currentId, "thread-b"];

    render(
      <ToolContent
        toolCalls={[makeToolCall("agent_finish", id)]}
        threadId="thread-b"
      />,
      { preloadedState: { chat } },
    );

    expect(screen.getByText("Right thread report")).toBeInTheDocument();
    expect(screen.queryByText("Wrong current report")).not.toBeInTheDocument();
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

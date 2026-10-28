import { describe, expect, it, vi } from "vitest";
import { within } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { applyChatEvent } from "../../features/Chat/Thread/actions";
import { selectCurrentThreadId } from "../../features/Chat/Thread/selectors";
import { DEDICATED_TOOL_CARD_NAMES } from "../Tools/AgenticTool/agenticFamilies";
import {
  launchPreviewCall,
  launchPreviewReceipt,
} from "../Tools/AgenticTool/launchFixtures";
import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
} from "../../utils/test-utils";
import type {
  BackgroundAgentSummary,
  PendingQuestionSet,
  ChatMessages,
  ExecToolMetadata,
  ToolCall,
  ToolEnrichment,
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
  enrichment?: ToolEnrichment,
): ToolMessage {
  return {
    role: "tool",
    tool_call_id: id,
    content,
    tool_failed: false,
    extra:
      extra ?? enrichment
        ? {
            ...(extra ? { exec: extra } : {}),
            ...(enrichment ? { tool_enrichment: enrichment } : {}),
          }
        : undefined,
  };
}

function renderToolContent(
  name: string,
  content: string,
  options: {
    args?: Record<string, unknown>;
    extra?: ExecToolMetadata;
    enrichment?: ToolEnrichment;
    resultExtra?: Record<string, unknown>;
    failed?: boolean;
    missing?: boolean;
    busy?: boolean;
    backgroundAgents?: Record<string, BackgroundAgentSummary>;
    pendingQuestions?: PendingQuestionSet[];
  } = {},
) {
  const id = `call-${name.replace(/[^a-z0-9]+/gi, "-")}`;
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
  if (!runtime) throw new Error("missing test thread");
  runtime.streaming = options.busy === true;
  runtime.waiting_for_response = false;
  runtime.background_agents = options.backgroundAgents ?? {};
  runtime.thread.pending_questions = options.pendingQuestions;
  const message = makeToolMessage(
    id,
    content,
    options.extra,
    options.enrichment,
  );
  if (options.resultExtra) message.extra = options.resultExtra;
  message.tool_failed = options.failed === true;
  runtime.thread.messages = options.missing ? [] : [message];

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
  it("missing results follow only their owning busy call group and remain unknown in idle or absent sessions", () => {
    const chat = createDefaultChatState();
    const currentThreadId = chat.current_thread_id;
    const generic = makeToolCall("unknown_tool", "pending-generic");
    const tasks = makeToolCall("tasks_set", "pending-tasks");
    const shell = makeToolCall("process_start", "pending-shell", {
      command: "echo ready",
    });
    const calls = [generic, tasks, shell];
    const runtime = chat.threads[currentThreadId];
    runtime.streaming = true;
    runtime.thread.messages = [
      { role: "assistant", content: "", tool_calls: calls },
    ];
    const owner = "rendered-owner";
    chat.threads[owner] = {
      ...runtime,
      thread: { ...runtime.thread, id: owner },
    };
    runtime.streaming = false;
    runtime.thread.messages = [
      makeToolMessage("pending-generic", "Other thread result"),
    ];
    const { store, container, rerender } = render(
      <ToolContent toolCalls={calls} threadId={owner} />,
      { preloadedState: { chat } },
    );
    expect(container.querySelectorAll('[data-status="running"]')).toHaveLength(
      3,
    );
    expect(screen.queryByText("Result not recorded")).toBeNull();
    act(() => {
      store.dispatch(
        applyChatEvent({
          type: "message_added",
          chat_id: owner,
          seq: "1",
          index: 1,
          message: makeToolMessage("pending-generic", "Recorded live result"),
        }),
      );
    });
    expect(container.querySelectorAll('[data-status="running"]')).toHaveLength(
      2,
    );
    expect(container.querySelectorAll('[data-status="success"]')).toHaveLength(
      1,
    );
    act(() => {
      store.dispatch(
        applyChatEvent({
          type: "runtime_updated",
          chat_id: owner,
          seq: "2",
          state: "idle",
        }),
      );
    });
    expect(container.querySelectorAll('[data-status="running"]')).toHaveLength(
      0,
    );
    expect(screen.getByText("Result not recorded")).toBeInTheDocument();
    expect(screen.getByTestId("exec-status-unknown")).toBeInTheDocument();
    act(() => {
      store.dispatch(
        applyChatEvent({
          type: "message_added",
          chat_id: owner,
          seq: "3",
          index: 2,
          message: {
            role: "assistant",
            content: "New response",
            tool_calls: [makeToolCall("unknown_tool", "new-call")],
          },
        }),
      );
      store.dispatch(
        applyChatEvent({
          type: "runtime_updated",
          chat_id: owner,
          seq: "4",
          state: "executing_tools",
        }),
      );
    });
    expect(container.querySelectorAll('[data-status="running"]')).toHaveLength(
      0,
    );
    rerender(<ToolContent toolCalls={calls} threadId="unknown-session" />);
    expect(container.querySelectorAll('[data-status="running"]')).toHaveLength(
      0,
    );
    expect(screen.getAllByText("Result not recorded")).toHaveLength(2);
  });

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
    const { container } = renderToolContent(name, content);

    expect(screen.getByTestId(testId)).toBeInTheDocument();
    if (name === "task_done") {
      expect(container.querySelector('[data-density="compact"]')).toBeNull();
      expect(screen.getByRole("button", { expanded: true })).toHaveTextContent(
        "Report unverified",
      );
    }
  });

  it.each(["structured", "legacy"])(
    "renders the full %s task_done report inline without opening a spoiler",
    async (format) => {
      const report = `# Full task report\n\n${"Evidence preserved. ".repeat(
        80,
      )}\n\nEND OF REPORT`;
      const content =
        format === "structured"
          ? JSON.stringify({
              type: "task_done",
              summary: "Task report received",
              report,
              files_changed: ["src/file.ts"],
            })
          : report;
      const { container } = renderToolContent("task_done", content);

      const heading = await screen.findByRole("heading", {
        name: "Full task report",
      });
      expect(heading).toBeVisible();
      expect(heading.closest("[inert], [aria-hidden='true']")).toBeNull();
      expect(screen.getByText("END OF REPORT")).toBeVisible();
      expect(screen.queryByText("More details")).not.toBeInTheDocument();
      expect(container.querySelector('[data-density="compact"]')).toBeNull();
      expect(
        container.querySelector("[class*='contentUnbounded']"),
      ).not.toBeNull();
      expect(screen.getByRole("button", { name: "Copy report" })).toBeVisible();
      if (format === "structured") {
        expect(screen.getByText("file.ts")).toBeVisible();
      }
    },
  );

  it("keeps agent diff specialized when native Git metadata is present", () => {
    renderToolContent("agent_diff", AGENT_DIFF_OUTPUT, {
      enrichment: {
        schema_version: 1,
        references: [
          {
            kind: "git",
            target: "workspace",
            provenance: "native",
            status: "available",
            details: { short_sha: "abc1234", scope: "card:T-54:stat" },
          },
        ],
      },
    });

    expect(screen.getByTestId("agent-diff-view")).toBeInTheDocument();
    expect(screen.queryByTestId("generic-tool")).not.toBeInTheDocument();
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
    "design_system",
    "ui_probe",
    "mark_elements",
    "contrast_audit",
    "image_region",
    "visual_diff",
  ])("routes %s to EngineAnalysisTool", (name) => {
    renderToolContent(
      name,
      JSON.stringify({ tool: name, summary: "Analysis" }),
    );

    expect(screen.getByTestId("engine-analysis-tool")).toBeInTheDocument();
    expect(screen.queryByTestId("generic-tool")).not.toBeInTheDocument();
  });

  it("renders design-tool image artifacts inside the analysis card", () => {
    renderToolContent(
      "visual_diff",
      JSON.stringify({
        tool: "visual_diff",
        summary: "Changed 2 pixels",
        changed_pixels: 2,
        changed_percent: 0.5,
        regions: [],
        artifact: {
          kind: "image",
          mime: "image/png",
          data: "cG5n",
          width: 2,
          height: 2,
          bytes: 3,
        },
      }),
    );

    openToolCard();
    expect(screen.getByTestId("artifacts-panel")).toBeInTheDocument();
  });

  it("renders security_scan summary instead of GenericTool fallback", () => {
    renderToolContent(
      "security_scan",
      JSON.stringify({
        tool: "security_scan",
        summary: "Security scan",
        path: "src/server.ts",
        lang: "TypeScript",
        finding_count: 2,
        counts: { Critical: 1, High: 0, Medium: 1, Low: 0 },
        findings: [
          {
            rule: "dangerous-eval",
            severity: "Critical",
            line: 12,
            snippet: "eval(userInput)",
          },
          {
            rule: "insecure-random",
            severity: "Medium",
            line: 20,
            snippet: "Math.random()",
          },
        ],
        omitted: 0,
      }),
    );

    expect(screen.getByTestId("engine-analysis-tool")).toBeInTheDocument();
    expect(screen.queryByTestId("generic-tool")).not.toBeInTheDocument();
    openToolCard();
    expect(screen.getAllByText("dangerous-eval").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Critical").length).toBeGreaterThan(0);
    expect(screen.getAllByText("eval(userInput)").length).toBeGreaterThan(0);
  });

  it("renders pr_blast summary with reviewers and impact kinds", () => {
    renderToolContent(
      "pr_blast",
      JSON.stringify({
        tool: "pr_blast",
        summary: "Blast radius",
        max_depth: 3,
        changed_files: ["src/main.rs", "src/router.ts"],
        directly_impacted: [
          {
            path: "src/app.rs",
            symbol: "renderApp",
            distance: 1,
            via: "calls",
            kind: "behavioral",
          },
        ],
        transitively_impacted: [
          {
            path: "src/state.ts",
            symbol: "createStore",
            distance: 2,
            via: "inherits",
            kind: "structural",
          },
        ],
        impacted_file_count: 2,
        risk_score: 0.62,
        suggested_reviewers: [{ author: "ada@example.com", score: 0.95 }],
        index_state: { queued: 3 },
        partial: true,
        warning: "index still building",
      }),
    );

    expect(screen.getByTestId("engine-analysis-tool")).toBeInTheDocument();
    expect(screen.queryByTestId("generic-tool")).not.toBeInTheDocument();
    openToolCard();
    expect(screen.getAllByText("index still building").length).toBeGreaterThan(
      0,
    );
    expect(screen.getAllByText("structural").length).toBeGreaterThan(0);
    expect(screen.getAllByText(/ada@example.com/).length).toBeGreaterThan(0);
  });

  it("renders dead_code confidence preview", () => {
    renderToolContent(
      "dead_code",
      JSON.stringify({
        tool: "dead_code",
        summary: "Dead code",
        entries: [
          {
            name: "unusedHelper",
            path: "src/unused.ts",
            line: 42,
            reason: "unreachable",
            confidence: 0.91,
            git_recency: 400,
            incoming_edges: 0,
          },
        ],
        shown: 1,
        total_candidates: 4,
        index_state: {
          queued: 0,
          dirty_paths: 0,
          pending_refs: 0,
          cross_file_edges: 10,
          cross_file_ready: true,
        },
        partial: false,
      }),
    );

    expect(screen.getByTestId("engine-analysis-tool")).toBeInTheDocument();
    expect(screen.queryByTestId("generic-tool")).not.toBeInTheDocument();
    openToolCard();
    expect(screen.getAllByText("unusedHelper").length).toBeGreaterThan(0);
    expect(screen.getAllByText("0.91").length).toBeGreaterThan(0);
  });
  it("renders git_risk as a structured report instead of a raw text dump", () => {
    renderToolContent(
      "git_risk",
      JSON.stringify({
        tool: "git_risk",
        summary: "Git risk",
        commits_analyzed: 1000,
        agent_authored_pct: 0.02,
        hotspots: [
          {
            path: "src/app.rs",
            churn: 67,
            risk: 0.8,
            churn_risk: 0.8,
            churn_percentile: 1,
            temporal_score: 106.2,
            change_entropy: 1,
            change_entropy_pct: 1,
            bus_factor: 2,
            ownership_risk: true,
            knowledge_loss: false,
          },
        ],
        ownership: [],
        co_change: [],
        coupling: [],
        reviewers: [],
        findings: [
          {
            path: "src/app.rs",
            biomarker: "change_entropy",
            category: "git",
            dimension: "defect",
            severity: "Critical",
            line: 1,
            detail: "change entropy pct 1.00 across 67 commits",
          },
        ],
        recent_commit_risks: [],
      }),
    );

    openToolCard();
    expect(screen.getByTestId("analysis-report")).toBeInTheDocument();
    expect(screen.getAllByText("churn 67").length).toBeGreaterThan(0);
    expect(screen.getAllByText("ownership-risk").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Critical").length).toBeGreaterThan(0);
    expect(
      screen.getAllByText("change entropy pct 1.00 across 67 commits").length,
    ).toBeGreaterThan(0);
  });

  it("renders codegraph_overview counts as a metric grid", () => {
    renderToolContent(
      "codegraph_overview",
      JSON.stringify({
        tool: "codegraph_overview",
        summary: "Code graph overview",
        counts: { nodes: 100205, edges: 212738, files: 20 },
        index_state: {
          queued: 1,
          cross_file_edges: 5,
          cross_file_ready: false,
        },
        scc_count: 2,
        largest_scc: 10,
        component_count: 3,
        top_pagerank: [],
        top_betweenness: [],
        file_centrality: { top_pagerank: [], top_betweenness: [] },
        community_count: 2,
        dead_code_count: 1,
        partial: true,
        communities: [],
        execution_flows: [],
        dead_code: [],
        entry_points: ["src/main.rs"],
        api_contract_files: [],
      }),
    );

    openToolCard();
    expect(screen.getByTestId("analysis-report")).toBeInTheDocument();
    expect(screen.getAllByText("Nodes").length).toBeGreaterThan(0);
    expect(screen.getAllByText("100205").length).toBeGreaterThan(0);
    expect(screen.getAllByText("queued 1").length).toBeGreaterThan(0);
  });

  it("renders non-JSON engine errors without crashing", () => {
    renderToolContent("dead_code", "engine unavailable");

    openToolCard();
    expect(screen.queryByTestId("analysis-report")).not.toBeInTheDocument();
    expect(screen.getByText("engine unavailable")).toBeInTheDocument();
  });

  it("expands a long section on demand", () => {
    const rows = Array.from({ length: 15 }, (_, index) => ({
      path: `src/a${index}.ts`,
      biomarker: `entry-${index}`,
      category: "dry",
      dimension: "maintainability",
      severity: "Low",
      line: index + 1,
      detail: `value-${index}`,
    }));
    renderToolContent(
      "code_duplication",
      JSON.stringify({
        tool: "code_duplication",
        summary: "Duplication",
        aggregate: {
          file_count: 15,
          clone_pair_count: 0,
          duplication_pct: 0,
          duplication_percent: 0,
        },
        clones: [],
        dry_violations: rows,
        test_smells: [],
      }),
    );

    openToolCard();
    expect(screen.queryByText("entry-14")).not.toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("button", { name: "Show 3 more in DRY violations" }),
    );
    expect(screen.getByText("entry-14")).toBeInTheDocument();
  });

  it("routes plain-text agent_finish results through FinalReportView legacy fallback", () => {
    renderToolContent("agent_finish", "Plain legacy report");

    expect(screen.queryByTestId("generic-tool")).not.toBeInTheDocument();
    expect(screen.queryByTestId("final-report-view")).not.toBeInTheDocument();
    expect(screen.getByTestId("final-report-tool")).toBeInTheDocument();
    openToolCard();
    fireEvent.click(screen.getByRole("button", { name: "More details" }));
    expect(
      screen.getByText("Plain legacy report", { selector: "p" }),
    ).toBeInTheDocument();
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
    openToolCard();
    expect(screen.getByText("exec_service_dev")).toBeInTheDocument();
  });

  it("renders valid enrichment on generic tools without changing the raw fallback", () => {
    renderToolContent("unknown_tool", "raw legacy fallback", {
      enrichment: {
        schema_version: 1,
        references: [
          {
            kind: "path",
            target: "src/lib.rs",
            label: "lib.rs",
            provenance: "native",
            line1: 2,
            line2: 4,
            status: "updated",
          },
        ],
      },
    });

    openToolCard();
    expect(screen.getByTestId("tool-enrichment")).toBeInTheDocument();
    expect(screen.getByText("path: lib.rs:2-4 (updated)")).toBeInTheDocument();
    expect(screen.getByText("raw legacy fallback")).toBeInTheDocument();
  });

  it("ignores unknown enrichment versions and kinds safely", () => {
    renderToolContent("unknown_tool", "raw legacy fallback", {
      enrichment: {
        schema_version: 2,
        references: [
          {
            kind: "path",
            target: "src/lib.rs",
            provenance: "native",
          },
        ],
      } as unknown as ToolEnrichment,
    });

    openToolCard();
    expect(screen.queryByTestId("tool-enrichment")).not.toBeInTheDocument();
    expect(screen.getByText("raw legacy fallback")).toBeInTheDocument();
  });

  it("keeps specialized cards ahead of generic enrichment rendering", () => {
    renderToolContent("process_start", "Process started", {
      enrichment: {
        schema_version: 1,
        references: [
          {
            kind: "process",
            target: "process-1",
            provenance: "native",
          },
        ],
      },
    });

    expect(screen.getByTestId("exec-tool-process_start")).toBeInTheDocument();
    expect(screen.queryByTestId("generic-tool")).not.toBeInTheDocument();
    expect(screen.queryByTestId("tool-enrichment")).not.toBeInTheDocument();
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

    openToolCard();
    fireEvent.click(screen.getByRole("button", { name: "More details" }));
    expect(
      screen.getByText("Right thread report", { selector: "p" }),
    ).toBeInTheDocument();
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
    expect(screen.getAllByText("Reported failure")).not.toHaveLength(0);
  });

  it("final_report_tool_card_shows_success_when_success_true", () => {
    renderToolContent("agent_finish", structuredFinalReport(true));

    expect(
      screen.getByTestId("final-report-tool-success-icon"),
    ).toBeInTheDocument();
    expect(
      screen.queryByTestId("final-report-tool-error-icon"),
    ).not.toBeInTheDocument();
    expect(screen.getByText("Reported success")).toBeInTheDocument();
  });
});

const DEDICATED_COMPACT_NAMES = [
  ...DEDICATED_TOOL_CARD_NAMES.filter((name) => name !== "task_done"),
  "ask_questions",
  "ask",
  "sleep",
];

describe("compact dedicated transcript composition", () => {
  it.each(DEDICATED_COMPACT_NAMES)(
    "%s and aliases never fall through to generic and begin open only for unanswered questions",
    (name) => {
      const pascal = name
        .split("_")
        .map((part) => part[0].toUpperCase() + part.slice(1))
        .join("");
      for (const alias of [name, `t_${name}`, pascal]) {
        for (const missing of [true, false]) {
          const { container, unmount } = renderToolContent(
            alias,
            "RAW_TECHNICAL_SENTINEL: not recognized",
            {
              missing,
              args: {
                task: "Inspect the scoped change",
                slug: "main-plan",
                description: "Wait for evidence",
                duration_ms: 1000,
              },
            },
          );
          expect(screen.queryByTestId("generic-tool"), alias).toBeNull();
          if (name === "ask_questions" || name === "ask") {
            expect(
              screen.getByTestId("ask-questions-tool"),
              alias,
            ).toBeInTheDocument();
            unmount();
            continue;
          }
          const row = container.querySelector(
            'section[data-density="compact"]',
          );
          expect(row, alias).not.toBeNull();
          expect(row).toHaveAttribute("data-open", "false");
          expect(row?.querySelector("button")).not.toHaveTextContent(
            "RAW_TECHNICAL_SENTINEL",
          );
          expect(row?.querySelector("button")).toHaveAttribute(
            "aria-expanded",
            "false",
          );
          unmount();
        }
      }
    },
  );

  it.each(["doc_get", "doc_list", "agent_finish", "subagent"])(
    "%s missing historical output is idle, not success or running",
    (name) => {
      const { container } = renderToolContent(name, "", { missing: true });
      expect(container.querySelector("section")).toHaveAttribute(
        "data-status",
        "idle",
      );
      expect(screen.queryByTestId("generic-tool")).toBeNull();
      expect(container.querySelector(".lucide-loader-circle")).toBeNull();
      expect(container.querySelector("button")).toHaveAttribute(
        "aria-expanded",
        "false",
      );
    },
  );

  it("a genuinely busy legacy launch can show request activity without inventing child state", () => {
    const { container } = renderToolContent("subagent", "", {
      missing: true,
      busy: true,
      args: { task: "Inspect the scoped change" },
    });
    expect(container.querySelector("section")).toHaveAttribute(
      "data-status",
      "running",
    );
    expect(screen.getByRole("button")).toHaveTextContent("Awaiting report");
    expect(screen.getByRole("button")).not.toHaveTextContent(
      /queued|completed|verified/iu,
    );
  });

  it.each([
    undefined,
    "future_state",
    "queued",
    "running",
    "waiting_for_approval",
    "completed",
    "failed",
  ])(
    "launch status %s is evidence-limited and raw output is only deeper",
    (status) => {
      const { container } = renderToolContent(
        "subagent",
        "RAW_LAUNCH_ONLY_ONCE",
        {
          args: { task: "Inspect scoped files" },
          resultExtra: {
            background_agent_id: "bg-child",
            child_chat_id: "child-chat",
            ...(status ? { background_agent_status: status } : {}),
          },
        },
      );
      const row = screen.getByTestId("background-agent-inline");
      const outer = within(row).getAllByRole("button")[0];
      expect(outer).toHaveAttribute("aria-expanded", "false");
      expect(outer).toHaveTextContent("Inspect scoped files");
      expect(outer).toHaveTextContent(
        !status || status === "future_state"
          ? "Status unknown"
          : status === "waiting_for_approval"
            ? "Approval needed"
            : status[0].toUpperCase() + status.slice(1),
      );
      expect(outer).not.toHaveTextContent("RAW_LAUNCH_ONLY_ONCE");
      expect(
        container.querySelectorAll('section[data-density="compact"]'),
      ).toHaveLength(1);
      const raw = screen.getByLabelText("Full response content");
      expect(raw.closest("[inert]")).not.toBeNull();
      fireEvent.click(outer);
      expect(raw.closest("[inert]")).not.toBeNull();
      fireEvent.click(screen.getByRole("button", { name: "More details" }));
      expect(raw.closest("[inert]")).toBeNull();
      expect(screen.getAllByText("RAW_LAUNCH_ONLY_ONCE")).toHaveLength(1);
      expect(container.querySelector("button button")).toBeNull();
    },
  );

  it.each([false, true])(
    "failed launch keeps request failure separate from child state with subscribed=%s",
    (subscribed) => {
      const child = {
        agent_id: "bg-child",
        parent_chat_id: "parent",
        child_chat_id: "child-chat",
        kind: "subagent",
        status: "running",
        title: "Live work",
        progress: "Reading files",
        step_count: 1,
        last_activity: null,
        target_files: [],
        edited_files: [],
        diff_summary: null,
        conflict_summary: null,
        result_summary: null,
        error: null,
        started_at: null,
        finished_at: null,
        change_seq: 1,
      } satisfies BackgroundAgentSummary;
      const { store } = renderToolContent(
        "subagent",
        "Request transport failed",
        {
          failed: true,
          resultExtra: {
            background_agent_id: "bg-child",
            child_chat_id: "child-chat",
            background_agent_status: "running",
          },
          backgroundAgents: subscribed ? { "bg-child": child } : {},
        },
      );
      const row = screen.getByTestId("background-agent-inline");
      expect(row).toHaveAttribute("data-status", "error");
      expect(screen.getByRole("alert")).toHaveTextContent("Request failed");
      expect(within(row).getAllByRole("button")[0]).not.toHaveTextContent(
        "Failed child",
      );
      fireEvent.click(within(row).getAllByRole("button")[0]);
      fireEvent.click(screen.getByRole("button", { name: "More details" }));
      expect(screen.getByText("Running")).toBeInTheDocument();
      expect(screen.getByLabelText("Full response content")).toHaveTextContent(
        "Request transport failed",
      );
      fireEvent.click(screen.getByRole("button", { name: "Open trajectory" }));
      expect(selectCurrentThreadId(store.getState())).toBe("child-chat");
      expect(child.status).toBe("running");
    },
  );

  it("preview remains a single preview row and cannot become a live launch card", () => {
    const chat = createDefaultChatState();
    const receipt = launchPreviewReceipt();
    const call = {
      ...launchPreviewCall,
      function: {
        ...launchPreviewCall.function,
        arguments: JSON.stringify({
          action: "preview",
          task: `Inspect accepted context ${"x".repeat(9000)}ARGUMENT_TAIL`,
        }),
      },
    };
    receipt.content = `${"x".repeat(9000)}RESPONSE_TAIL`;
    receipt.extra = {
      ...receipt.extra,
      diagnostic: `${"x".repeat(9000)}METADATA_TAIL`,
    };
    chat.threads[chat.current_thread_id].thread.messages = [receipt];
    const { container } = render(<ToolContent toolCalls={[call]} />, {
      preloadedState: { chat },
    });
    expect(screen.getByText("Launch blocked")).toBeInTheDocument();
    expect(screen.queryByTestId("background-agent-inline")).toBeNull();
    expect(screen.queryByLabelText("Full response content")).toBeNull();
    openToolCard();
    expect(screen.getByText("No child started")).toBeInTheDocument();
    const raw = screen.getByLabelText("Full response content");
    expect(raw.closest("[inert]")).not.toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "More details" }));
    expect(raw.closest("[inert]")).not.toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Technical details" }));
    expect(raw.closest("[inert]")).toBeNull();
    for (const [label, marker] of [
      ["Raw arguments", "ARGUMENT_TAIL"],
      ["Full response", "RESPONSE_TAIL"],
      ["Response metadata", "METADATA_TAIL"],
    ]) {
      const content = screen.getByLabelText(`${label} content`);
      expect(content).not.toHaveTextContent(marker);
      const next = within(screen.getByLabelText(`${label} pages`)).getByRole(
        "button",
        { name: "Next section" },
      );
      fireEvent.click(next);
      fireEvent.click(next);
      expect(content).toHaveTextContent(marker);
      expect(next).toBeDisabled();
    }
    expect(screen.getByText("Launch blocked")).toBeInTheDocument();
    expect(container.querySelector("button button")).toBeNull();
    expect(
      screen.queryByRole("button", {
        name: /^(?:launch|approve|run request)$/i,
      }),
    ).toBeNull();
  });

  it("protocol-v2 decisions stay expanded, preserving endpoint, instance and explicit submit", async () => {
    localStorage.clear();
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    try {
      const { user, container } = renderToolContent("ask_questions", "", {
        missing: true,
        pendingQuestions: [
          {
            tool_call_id: "call-ask-questions",
            asked_at_ms: 42,
            status: "pending",
            response_protocol_version: 2,
            questions: [
              { id: "one", type: "yes_no", text: "Proceed with the change?" },
            ],
          },
        ],
      });
      const card = container.querySelector("section");
      expect(card).toHaveTextContent("Needs you");
      expect(card).toHaveAttribute("data-open", "true");
      await user.click(screen.getByRole("radio", { name: "No" }));
      expect(fetchMock).not.toHaveBeenCalled();
      expect(container.querySelector("button button")).toBeNull();
      await user.click(screen.getByRole("button", { name: "Submit response" }));
      expect(fetchMock).toHaveBeenCalledOnce();
      const [url, request] = fetchMock.mock.calls[0] as [string, RequestInit];
      expect(url).toMatch(/\/v1\/chats\/[^/]+\/commands$/u);
      expect(JSON.parse(String(request.body))).toMatchObject({
        type: "submit_question_response",
        tool_call_id: "call-ask-questions",
        asked_at_ms: 42,
        response: {
          answers: [{ id: "one", disposition: "answered", answer: false }],
        },
      });
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it.each(["{broken", '{"success":true}', "Completed and verified!"])(
    "unknown final report %s stays neutral",
    (content) => {
      const { container } = renderToolContent("agent_finish", content);
      expect(
        screen.getByTestId("final-report-tool-neutral-icon"),
      ).toBeInTheDocument();
      expect(screen.queryByTestId("final-report-tool-success-icon")).toBeNull();
      expect(container.querySelector("section")).toHaveAttribute(
        "data-status",
        "idle",
      );
      expect(screen.getByText("Outcome unconfirmed")).toBeInTheDocument();
    },
  );

  it("final report disclosure keeps three facts, original request, metadata and complete report", () => {
    const report = `${STRUCTURED_FINAL_REPORT.slice(
      0,
      -1,
    )},"extra":"${"x".repeat(9000)}REPORT_TAIL"}`;
    const { container } = renderToolContent("agent_finish", report, {
      args: {
        success: true,
        report: `Original report argument ${"x".repeat(9000)}ARGUMENT_TAIL`,
      },
      resultExtra: {
        diagnostic: `Receipt metadata ${"x".repeat(9000)}METADATA_TAIL`,
      },
    });
    const outer = screen.getByRole("button", { expanded: false });
    expect(screen.queryByLabelText("Full response content")).toBeNull();
    fireEvent.click(outer);
    expect(container.querySelectorAll("dt")).toHaveLength(3);
    const more = screen.getByRole("button", { name: "More details" });
    expect(more).toHaveAttribute("aria-expanded", "false");
    const raw = screen.getByLabelText("Full response content");
    expect(raw.closest("[inert]")).not.toBeNull();
    fireEvent.click(more);
    expect(raw.closest("[inert]")).not.toBeNull();
    expect(screen.getByTestId("final-report-view")).toHaveTextContent(
      "npm run test -- ToolsContent --run",
    );
    const technical = screen.getAllByRole("button", {
      name: "Technical details",
    });
    fireEvent.click(technical[technical.length - 1]);
    expect(screen.getByLabelText("Raw arguments content")).toHaveTextContent(
      "Original report argument",
    );
    expect(
      screen.getByLabelText("Response metadata content"),
    ).toHaveTextContent("Receipt metadata");
    expect(raw.closest("[inert]")).toBeNull();
    for (const [label, marker] of [
      ["Raw arguments", "ARGUMENT_TAIL"],
      ["Full response", "REPORT_TAIL"],
      ["Response metadata", "METADATA_TAIL"],
    ]) {
      const content = screen.getByLabelText(`${label} content`);
      expect(content).not.toHaveTextContent(marker);
      const next = within(screen.getByLabelText(`${label} pages`)).getByRole(
        "button",
        { name: "Next section" },
      );
      fireEvent.click(next);
      fireEvent.click(next);
      expect(content).toHaveTextContent(marker);
      expect(next).toBeDisabled();
    }
    expect(container.querySelector("button button")).toBeNull();
  });

  it("final report result updates never force its explicit closed choice open", () => {
    const { store, container } = renderToolContent("agent_finish", "", {
      missing: true,
    });
    const outer = screen.getByRole("button", { expanded: false });
    const threadId = selectCurrentThreadId(store.getState());
    act(() => {
      store.dispatch(
        applyChatEvent({
          type: "message_added",
          chat_id: threadId,
          seq: "1",
          index: 0,
          message: {
            ...makeToolMessage(
              "call-agent-finish",
              structuredFinalReport(false),
            ),
            message_id: "report-result",
          },
        }),
      );
    });
    expect(outer).toHaveAttribute("aria-expanded", "false");
    expect(container.querySelector("section")).toHaveAttribute(
      "data-status",
      "error",
    );
  });
});

describe("neutral dedicated outcome regressions", () => {
  it.each(["task_done"])(
    "%s unknown results and idle missing results never imply success",
    (name) => {
      for (const missing of [false, true]) {
        const { container, unmount } = renderToolContent(
          name,
          "unverified prose",
          { missing },
        );
        expect(container.querySelector("section")).toHaveAttribute(
          "data-status",
          "idle",
        );
        expect(container.querySelector(".lucide-circle-check")).toBeNull();
        expect(container.querySelector(".lucide-loader-circle")).toBeNull();
        if (name === "task_done") {
          expect(
            screen.getByRole("button", { expanded: true }),
          ).toHaveTextContent(missing ? "State unknown" : "Report unverified");
          expect(
            container.querySelector('[data-density="compact"]'),
          ).toBeNull();
        }
        unmount();
      }
    },
  );

  it.each([
    ["task_done", true, "running"],
    ["agent_finish", true, "running"],
    ["agent_finish", false, "idle"],
  ] as const)(
    "%s absent result with busy=%s stays %s without promoting an outcome",
    (name, busy, status) => {
      const { container } = renderToolContent(name, "", {
        missing: true,
        busy,
      });
      expect(container.querySelector("section")).toHaveAttribute(
        "data-status",
        status,
      );
      expect(Boolean(container.querySelector(".lucide-loader-circle"))).toBe(
        busy,
      );
      expect(container.querySelector(".lucide-circle-check")).toBeNull();
      expect(screen.queryByTestId("generic-tool")).toBeNull();
      expect(container.querySelector("button")).toHaveAttribute(
        "aria-expanded",
        name === "task_done" ? "true" : "false",
      );
      if (name === "agent_finish") {
        expect(screen.getByRole("button")).toHaveTextContent("Awaiting report");
        expect(screen.getByRole("button")).not.toHaveTextContent(
          /queued|completed|verified|success/iu,
        );
      }
    },
  );

  it("agent_finish retains non-string multimodal history under technical details, never generic", () => {
    const content = [
      { m_type: "text", m_content: "Original multimodal report" },
      { m_type: "image/png", m_content: "abc123" },
    ];
    const id = "non-string-report";
    const chat = createDefaultChatState();
    chat.threads[chat.current_thread_id].thread.messages = [
      { role: "tool", tool_call_id: id, content, tool_failed: false },
    ];
    render(<ToolContent toolCalls={[makeToolCall("agent_finish", id)]} />, {
      preloadedState: { chat },
    });
    expect(screen.queryByTestId("generic-tool")).toBeNull();
    expect(
      screen.getByTestId("final-report-tool-neutral-icon"),
    ).toBeInTheDocument();
    expect(screen.getByText("Outcome unconfirmed")).toBeInTheDocument();
    openToolCard();
    fireEvent.click(screen.getByRole("button", { name: "More details" }));
    fireEvent.click(screen.getByRole("button", { name: "Technical details" }));
    expect(
      JSON.parse(
        screen.getByLabelText("Full response content").textContent ?? "",
      ),
    ).toEqual(content);
  });
});

describe("multimodal tool images", () => {
  it("renders real image parts and skips omitted placeholders", () => {
    const id = "mm-omitted";
    const chat = createDefaultChatState();
    chat.threads[chat.current_thread_id].thread.messages = [
      {
        role: "tool",
        tool_call_id: id,
        content: [
          { m_type: "text", m_content: "captured" },
          { m_type: "image/png", m_content: "<omitted>" },
          { m_type: "image/png", m_content: "iVBORw0KGgo=" },
        ],
        tool_failed: false,
      },
    ];
    const { container } = render(
      <ToolContent toolCalls={[makeToolCall("screenshot", id)]} />,
      { preloadedState: { chat } },
    );
    const sources = Array.from(container.querySelectorAll("img")).map(
      (image) => image.getAttribute("src") ?? "",
    );
    expect(sources).toEqual(["data:image/png;base64,iVBORw0KGgo="]);
  });
});

describe("removed review tool history", () => {
  it.each(["code_review", "review", "t_review"])(
    "%s historical messages fall back to the generic renderer",
    (name) => {
      const { container } = renderToolContent(name, "Original review receipt", {
        resultExtra: {
          review_report: { outcome: "inconclusive", findings: [] },
        },
      });
      expect(screen.getByTestId("generic-tool")).toBeInTheDocument();
      expect(container.textContent).not.toContain("Review unavailable");
    },
  );
});

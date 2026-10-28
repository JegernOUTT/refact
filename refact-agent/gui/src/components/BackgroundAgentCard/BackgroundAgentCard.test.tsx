import { describe, expect, it, vi, beforeEach } from "vitest";
import { act } from "react-dom/test-utils";
import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
} from "../../utils/test-utils";
import { BackgroundAgentCard } from "./BackgroundAgentCard";
import type {
  BackgroundAgentSummary,
  ChatMessage,
  ToolCall,
} from "../../services/refact/types";
import { applyChatEvent } from "../../features/Chat/Thread/actions";
import { ChatThreadProvider } from "../../features/Chat/Thread";
import { CollapsibleStoreProvider } from "../ChatContent/useStoredOpen";
import { ChatScrollAnchorContext } from "../ChatContent/useChatScrollAnchor";
import { SubagentTool } from "../ChatContent/ToolCard/SubagentTool";
import {
  agentFinished,
  agentResult,
  toolCallMessage,
} from "../../__fixtures__/foldedEvents";

const AGENT_ID = "bgagent-1f0c9a7b-3d21-4a55-9f0e-11223344abcd";
const ISO_TIMESTAMP = "2026-02-11T09:14:22.123456789Z";

function makeAgent(
  overrides: Partial<BackgroundAgentSummary> = {},
): BackgroundAgentSummary {
  return {
    agent_id: AGENT_ID,
    parent_chat_id: "parent-chat",
    child_chat_id: "child-chat",
    kind: "delegate",
    status: "running",
    title: "Redesign the background agent card",
    progress: "Updating components",
    step_count: 19,
    last_activity: new Date(Date.now() - 2 * 60_000).toISOString(),
    target_files: [
      "refact-agent/gui/src/components/BackgroundAgentCard/BackgroundAgentCard.tsx",
      "refact-agent/gui/src/components/BackgroundAgentCard/BackgroundAgentCard.module.css",
    ],
    edited_files: [],
    diff_summary: null,
    conflict_summary: null,
    result_summary: null,
    error: null,
    started_at: ISO_TIMESTAMP,
    finished_at: null,
    change_seq: 3,
    model: "openai/gpt-5.6-terra",
    model_type: "thinking",
    current_tool: "shell: cargo test --lib background_agent",
    goal_summary: "Ship the compact expandable background-agent card",
    plan_present: true,
    worktree_branch: "refact/task/T-8/card",
    merge_status: "pending",
    pending_questions: 1,
    questions: [
      {
        id: "question-1",
        text: "Should the card start expanded?",
        asked_at: ISO_TIMESTAMP,
      },
      {
        id: "question-2",
        text: "Can the branch be merged?",
        answer: "Yes, once the GUI checks pass.",
        asked_at: ISO_TIMESTAMP,
        answered_at: ISO_TIMESTAMP,
      },
    ],
    tokens_used: 12_300,
    cost_usd: 0.04,
    ...overrides,
  };
}

const writeText = vi.fn();

beforeEach(() => {
  writeText.mockClear();
  Object.defineProperty(window.navigator.clipboard, "writeText", {
    configurable: true,
    value: writeText,
  });
});

function toggleOf(root: HTMLElement): HTMLElement {
  const toggle = root.querySelector<HTMLElement>(
    '[data-testid="transcript-toggle"]',
  );
  if (!toggle) throw new Error("Missing transcript toggle");
  return toggle;
}

function factValue(label: string): HTMLElement | null {
  const term = screen
    .queryAllByRole("term")
    .find((node) => node.textContent === label);
  return term?.nextElementSibling instanceof HTMLElement
    ? term.nextElementSibling
    : null;
}

describe("BackgroundAgentCard", () => {
  it("renders the panel card as a single transcript item", () => {
    const { container } = render(
      <BackgroundAgentCard
        agent={makeAgent({ pending_questions: 0, questions: [] })}
      />,
    );
    const root = screen.getByTestId("background-agent-card");
    expect(root).toHaveAttribute("data-transcript-item", "true");
    expect(root).toHaveAttribute("data-open", "false");
    expect(root).toHaveAttribute("data-agent-kind", "delegate");
    expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Delegate");
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Redesign the background agent card",
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent("step 19");
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Running",
    );
    expect(
      container.querySelectorAll('[data-transcript-item="true"]'),
    ).toHaveLength(1);
  });

  it("starts open when explicitly requested by panel callers", () => {
    render(
      <BackgroundAgentCard
        presentation="panel"
        compactDefault={false}
        agent={makeAgent({ pending_questions: 0 })}
      />,
    );
    expect(screen.getByTestId("background-agent-card")).toHaveAttribute(
      "data-open",
      "true",
    );
    expect(screen.queryByTestId("background-agent-inline")).toBeNull();
  });

  it("opens for unanswered questions and labels the attention state", () => {
    render(<BackgroundAgentCard agent={makeAgent()} />);
    expect(screen.getByTestId("background-agent-card")).toHaveAttribute(
      "data-open",
      "true",
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "1 question",
    );
    const qa = screen.getByRole("region", {
      name: "Agent questions and answers",
    });
    expect(qa).toHaveTextContent("Q: Should the card start expanded?");
    expect(qa).toHaveTextContent("Awaiting reply");
    expect(qa).toHaveTextContent("A: Yes, once the GUI checks pass.");
  });

  it("puts model, worktree, merge and files into summary facts", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({
          status: "completed",
          kind: "subagent",
          merge_status: "conflict",
          conflict_summary: "src/a.ts conflicts",
          edited_files: ["src/a.ts", "src/b.ts"],
          diff_summary: "+42 -7",
          pending_questions: 0,
          questions: [],
        })}
      />,
    );
    fireEvent.click(toggleOf(screen.getByTestId("background-agent-card")));
    expect(factValue("Status")).toHaveTextContent("Conflict");
    expect(factValue("Status")).toHaveAttribute("title", "src/a.ts conflicts");
    expect(factValue("Model")).toHaveTextContent("openai/gpt-5.6-terra");
    expect(factValue("Worktree")).toHaveTextContent("refact/task/T-8/card");
    expect(factValue("Files changed")).toHaveTextContent("2 · +42 -7");
    expect(factValue("Verification")).toHaveTextContent("Report unverified");
    expect(screen.getByText("a.ts")).toBeInTheDocument();
    expect(screen.getByText("b.ts")).toBeInTheDocument();
    expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Subagent");
  });

  it("renders the structural deliverable verdict as a fact with reasons", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({
          status: "completed",
          pending_questions: 0,
          deliverable_status: {
            met: false,
            check: "structural",
            missing_sections: ["Evidence"],
            schema_error: null,
            evidence_missing: true,
          },
        })}
      />,
    );
    fireEvent.click(toggleOf(screen.getByTestId("background-agent-card")));
    const value = factValue("Verification");
    expect(value).toHaveTextContent("Deliverable unmet");
    expect(value?.getAttribute("title")).toContain(
      "missing sections: Evidence",
    );
  });

  it("distinguishes a declared but unevaluated deliverable contract", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({
          status: "running",
          pending_questions: 0,
          deliverable: {
            required_sections: [],
            output_schema: null,
            evidence_required: false,
          },
        })}
      />,
    );
    fireEvent.click(toggleOf(screen.getByTestId("background-agent-card")));
    expect(factValue("Verification")).toHaveTextContent(
      "Deliverable not evaluated",
    );
  });

  it("shows step count, duration and cost for terminal agents", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({
          status: "completed",
          pending_questions: 0,
          started_at: "2026-02-11T09:00:00Z",
          finished_at: "2026-02-11T09:02:00Z",
          cost_usd: 0.04,
        })}
      />,
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "19 steps · 2.0min · $0.04",
    );
    expect(screen.queryByTestId("transcript-status-pill")).toBeNull();
  });

  it("renders the result report as markdown and one trajectory action", () => {
    const onOpenTrajectory = vi.fn();
    render(
      <BackgroundAgentCard
        onOpenTrajectory={onOpenTrajectory}
        agent={makeAgent({
          status: "completed",
          pending_questions: 0,
          result_summary: "## Findings\n\n- first point\n- second point",
        })}
      />,
    );
    fireEvent.click(toggleOf(screen.getByTestId("background-agent-card")));
    expect(
      screen.getByRole("heading", { name: "Findings" }),
    ).toBeInTheDocument();
    expect(screen.getByText("first point").tagName).toBe("LI");
    expect(screen.getByRole("group", { name: "Report" })).toBeInTheDocument();
    const open = screen.getAllByRole("button", { name: "Open trajectory" });
    expect(open).toHaveLength(1);
    fireEvent.click(open[0]);
    expect(onOpenTrajectory).toHaveBeenCalledWith("child-chat");
  });

  it("hides the trajectory action without a child chat", () => {
    render(
      <BackgroundAgentCard
        compactDefault={false}
        onOpenTrajectory={vi.fn()}
        agent={makeAgent({ child_chat_id: null })}
      />,
    );
    expect(
      screen.queryByRole("button", { name: "Open trajectory" }),
    ).toBeNull();
  });

  it("shows live progress for running agents", () => {
    const { rerender } = render(
      <BackgroundAgentCard
        compactDefault={false}
        agent={makeAgent({ pending_questions: 0 })}
      />,
    );
    expect(screen.getByRole("group", { name: "Progress" })).toHaveTextContent(
      "Updating components",
    );
    rerender(
      <BackgroundAgentCard
        compactDefault={false}
        agent={makeAgent({ pending_questions: 0, progress: "Running tests" })}
      />,
    );
    expect(screen.getByRole("group", { name: "Progress" })).toHaveTextContent(
      "Running tests",
    );
    expect(factValue("Now")).toHaveTextContent(
      "shell: cargo test --lib background_agent",
    );
  });

  it("keeps the agent id in raw payloads and never renders raw timestamps", () => {
    const { container } = render(
      <BackgroundAgentCard compactDefault={false} agent={makeAgent()} />,
    );
    expect(container.textContent).not.toContain(ISO_TIMESTAMP);
    fireEvent.click(screen.getByRole("button", { name: /^Raw/ }));
    expect(screen.getByTestId("transcript-raw")).toHaveTextContent(AGENT_ID);
  });
});

describe("inline background agents", () => {
  it.each([
    ["running", "Running"],
    ["queued", "Queued"],
    ["waiting_for_approval", "Approval needed"],
    ["failed", "Failed"],
    ["interrupted", "Interrupted"],
    ["cancelled", "Cancelled"],
  ] as const)("maps authoritative %s status to the pill", (status, label) => {
    render(
      <BackgroundAgentCard
        presentation="inline"
        agent={makeAgent({
          status,
          title: "Subagent: Inspect the animation",
          pending_questions: 0,
        })}
      />,
    );
    const row = screen.getByTestId("background-agent-inline");
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Inspect the animation",
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      label,
    );
    expect(row).toHaveAttribute(
      "data-open",
      status === "waiting_for_approval" ? "true" : "false",
    );
  });

  it("hides the pill for completed agents", () => {
    render(
      <BackgroundAgentCard
        presentation="inline"
        agent={makeAgent({ status: "completed", pending_questions: 0 })}
      />,
    );
    expect(screen.queryByTestId("transcript-status-pill")).toBeNull();
  });

  it("uses neutral unknown for a launch placeholder rather than synthetic queued", () => {
    render(
      <BackgroundAgentCard
        presentation="inline"
        statusKnown={false}
        agent={makeAgent({
          status: "queued",
          progress: "Paused; completed; verified and pushed",
          pending_questions: 0,
        })}
      />,
    );
    const pill = screen.getByTestId("transcript-status-pill");
    expect(pill).toHaveTextContent("Status unknown");
    expect(pill).toHaveAttribute("data-tone", "unknown");
    expect(screen.getByTestId("transcript-row")).not.toHaveTextContent(
      /paused|verified|pushed/i,
    );
  });

  it("does not upgrade an unrecognized live status over a neutral placeholder", () => {
    const chat = createDefaultChatState();
    const threadId = chat.current_thread_id;
    const agent = makeAgent({ pending_questions: 0 });
    Object.defineProperty(agent, "status", { value: "paused" });
    chat.threads[threadId].background_agents = { [agent.agent_id]: agent };
    render(
      <BackgroundAgentCard
        presentation="inline"
        statusKnown={false}
        threadId={threadId}
        agent={makeAgent({ status: "queued" })}
      />,
      { preloadedState: { chat } },
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Status unknown",
    );
  });

  it("shows the full task instead of an engine-truncated title", () => {
    const task =
      "Implement approved Buddy HOME overhaul in active worktree and verify the GUI checks";
    const call: ToolCall = {
      id: "truncated-title",
      index: 0,
      function: { name: "subagent", arguments: JSON.stringify({ task }) },
    };
    render(
      <BackgroundAgentCard
        presentation="inline"
        toolCall={call}
        agent={makeAgent({
          title:
            "Subagent: Implement approved Buddy HOME overhaul in active w…",
          pending_questions: 0,
        })}
      />,
    );
    const target = screen.getByTestId("transcript-target");
    expect(target).toHaveTextContent(task);
    expect(target).not.toHaveTextContent(/subagent|…/i);
  });

  it("strips a repeated agent kind prefix from the target", () => {
    render(
      <BackgroundAgentCard
        presentation="inline"
        agent={makeAgent({
          title: "Subagent: Subagent: Implement X",
          pending_questions: 0,
        })}
      />,
    );
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      /^Implement X$/,
    );
  });

  it("escapes a hostile title", () => {
    render(
      <BackgroundAgentCard
        presentation="inline"
        agent={makeAgent({
          title: `<script>attack()</script> ${"🦊".repeat(300)}`,
          pending_questions: 0,
        })}
      />,
    );
    const row = screen.getByTestId("background-agent-inline");
    expect(row.querySelector("script")).toBeNull();
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "<script>attack()</script>",
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Running",
    );
  });

  it("renders task input, launch result and raw launch payload", async () => {
    const chat = createDefaultChatState();
    const threadId = chat.current_thread_id;
    const call: ToolCall = {
      id: "launch-call",
      index: 0,
      function: {
        name: "subagent",
        arguments: JSON.stringify({ task: "Review **the** parser" }),
      },
    };
    chat.threads[threadId].thread.messages = [
      {
        role: "tool",
        tool_call_id: "launch-call",
        content: "✓ Accepted background subagent",
        tool_failed: false,
        extra: { background_agent_id: AGENT_ID },
      },
    ];
    const { user } = render(
      <BackgroundAgentCard
        presentation="inline"
        threadId={threadId}
        toolCall={call}
        agent={makeAgent({ pending_questions: 0 })}
      />,
      { preloadedState: { chat } },
    );
    await user.click(toggleOf(screen.getByTestId("background-agent-inline")));
    const task = screen.getByRole("group", { name: "Task" });
    expect(task.querySelector("strong")).toHaveTextContent("the");
    await user.click(screen.getByRole("button", { name: /^Raw/ }));
    const raw = screen.getByTestId("transcript-raw");
    expect(raw).toHaveTextContent("Launch arguments");
    expect(raw).toHaveTextContent("Accepted background subagent");
  });

  it("retains explicit open choices across remounts per tool call", () => {
    const values = new Map<string, boolean>([["tc:launch-one", true]]);
    const store = {
      get: (key: string) => values.get(key),
      set: (key: string, value: boolean) => values.set(key, value),
    };
    const view = (
      <CollapsibleStoreProvider value={store}>
        <BackgroundAgentCard
          presentation="inline"
          toolCallId="launch-one"
          agent={makeAgent({ pending_questions: 0 })}
        />
        <BackgroundAgentCard
          presentation="inline"
          toolCallId="launch-two"
          agent={makeAgent({ agent_id: "second", pending_questions: 0 })}
        />
      </CollapsibleStoreProvider>
    );
    const first = render(view);
    const rows = screen.getAllByTestId("background-agent-inline");
    expect(rows[0]).toHaveAttribute("data-open", "true");
    expect(rows[1]).toHaveAttribute("data-open", "false");
    fireEvent.click(toggleOf(rows[0]));
    first.unmount();
    render(view);
    const remounted = screen.getAllByTestId("background-agent-inline");
    expect(remounted[0]).toHaveAttribute("data-open", "false");
    expect(remounted[1]).toHaveAttribute("data-open", "false");
  });

  it("anchors inline expansion through the existing scroll context", () => {
    const preserve = vi.fn((mutate: () => void) => mutate());
    const prepare = vi.fn();
    render(
      <ChatScrollAnchorContext.Provider
        value={{ preserveScrollAnchor: preserve, prepareScrollAnchor: prepare }}
      >
        <BackgroundAgentCard
          presentation="inline"
          agent={makeAgent({ pending_questions: 0 })}
        />
      </ChatScrollAnchorContext.Provider>,
    );
    const toggle = toggleOf(screen.getByTestId("background-agent-inline"));
    fireEvent.pointerDown(toggle);
    fireEvent.click(toggle);
    expect(prepare).toHaveBeenCalled();
    expect(preserve).toHaveBeenCalled();
  });

  it("follows live subscription updates without reopening", () => {
    const chat = createDefaultChatState();
    const threadId = chat.current_thread_id;
    const agent = makeAgent({ status: "queued", pending_questions: 0 });
    const { store } = render(
      <BackgroundAgentCard
        presentation="inline"
        agent={agent}
        threadId={threadId}
        statusKnown={false}
      />,
      { preloadedState: { chat } },
    );
    const row = screen.getByTestId("background-agent-inline");
    const update = (seq: number, overrides: Partial<BackgroundAgentSummary>) =>
      act(() => {
        store.dispatch(
          applyChatEvent({
            chat_id: threadId,
            seq: String(seq),
            type: "background_agent_updated",
            agent: { ...agent, change_seq: seq, ...overrides },
          }),
        );
      });
    update(1, { status: "running", title: "Live title" });
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Live title",
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Running",
    );
    expect(row).toHaveAttribute("data-open", "false");
    update(2, { status: "failed", error: "new failure" });
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Failed",
    );
    expect(row).toHaveAttribute("data-open", "false");
  });
});

describe("legacy SubagentTool", () => {
  const call: ToolCall = {
    id: "legacy-report",
    index: 0,
    function: {
      name: "subagent",
      arguments: JSON.stringify({
        task: "Inspect animation",
        model_name: "private-model",
        target_files: ["src/a.ts"],
      }),
    },
  };
  const legacy =
    "# Subagent Result\n\n**Task:** Inspect animation\n\n## Response\n\n## Findings\n\n- Animation findings preserved.";

  function renderLegacy(
    content?: string,
    failed = false,
    argumentsValue = call.function.arguments,
  ) {
    const chat = createDefaultChatState();
    const threadId = chat.current_thread_id;
    const messages: ChatMessage[] =
      content === undefined
        ? []
        : [
            {
              role: "tool",
              content,
              tool_call_id: call.id ?? "",
              tool_failed: failed,
            },
          ];
    chat.threads[threadId].thread.messages = messages;
    return render(
      <ChatThreadProvider chatId={threadId}>
        <SubagentTool
          toolCall={{
            ...call,
            function: { ...call.function, arguments: argumentsValue },
          }}
        />
      </ChatThreadProvider>,
      { preloadedState: { chat } },
    );
  }

  it("renders one item with the report as real markdown", async () => {
    const { container, user } = renderLegacy(legacy);
    expect(
      container.querySelectorAll('[data-transcript-item="true"]'),
    ).toHaveLength(1);
    expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Subagent");
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Inspect animation",
    );
    await user.click(screen.getByTestId("transcript-toggle"));
    expect(factValue("Model")).toHaveTextContent("private-model");
    expect(factValue("Target files")).toHaveTextContent("1");
    expect(factValue("Verification")).toHaveTextContent(
      "No verification evidence",
    );
    expect(
      screen.getByRole("heading", { name: "Findings" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Animation findings preserved.").tagName).toBe(
      "LI",
    );
    expect(container.textContent).not.toContain("Technical details");
    expect(
      screen.getByRole("button", { name: "Copy report" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^Raw/ }));
    expect(screen.getByTestId("transcript-raw")).toHaveTextContent(
      "# Subagent Result",
    );
  });

  it.each([
    [undefined, false, "No result"],
    ["Launch failed", true, "Request failed"],
  ] as const)("does not infer lifecycle from %s", (content, failed, label) => {
    renderLegacy(content, failed);
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      label,
    );
  });

  it.each(["null", "[]", "broken", '{"task": {"text": "not scalar"}}'])(
    "handles malformed legacy arguments %s",
    (args) => {
      renderLegacy(undefined, false, args);
      expect(screen.getByTestId("transcript-target")).toHaveTextContent(
        "Background work",
      );
    },
  );
});

describe("inline request failure presentation", () => {
  it("keeps launch failure independent of a running child", () => {
    const agent = makeAgent({ status: "running", pending_questions: 0 });
    const { rerender } = render(
      <BackgroundAgentCard presentation="inline" launchFailed agent={agent} />,
    );
    const row = screen.getByTestId("background-agent-inline");
    expect(row).toHaveAttribute("data-status", "failed");
    expect(row).toHaveAttribute("data-open", "false");
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Request failed",
    );
    rerender(<BackgroundAgentCard launchFailed agent={agent} />);
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Running",
    );
  });
});

describe("BackgroundAgentCard folded finished event", () => {
  function renderFolded(withLiveAgent: boolean) {
    const chat = createDefaultChatState();
    const threadId = chat.current_thread_id;
    const launch = toolCallMessage("call-agent", "subagent", {
      task: "Investigate flaky test",
    });
    const call = launch.tool_calls?.[0];
    if (!call) throw new Error("Missing launch call");
    chat.threads[threadId].thread.messages = [
      launch,
      agentResult("call-agent", AGENT_ID),
      agentFinished(AGENT_ID),
    ];
    if (withLiveAgent) {
      chat.threads[threadId].background_agents = {
        [AGENT_ID]: makeAgent({
          status: "running",
          pending_questions: 0,
          questions: [],
        }),
      };
    }
    return render(
      <BackgroundAgentCard
        presentation="inline"
        threadId={threadId}
        toolCall={call}
        statusKnown={false}
        agent={makeAgent({
          status: "queued",
          pending_questions: 0,
          questions: [],
          result_summary: null,
          finished_at: null,
        })}
      />,
      { preloadedState: { chat } },
    );
  }

  it("falls back to the folded event when the live agent summary is missing", async () => {
    const { user } = renderFolded(false);
    const row = screen.getByTestId("background-agent-inline");

    expect(row).toHaveAttribute("data-status", "done");
    expect(screen.queryByText("Status unknown")).toBeNull();
    await user.click(toggleOf(row));
    expect(screen.getByText("All good, fixed it.")).toBeInTheDocument();
  });

  it("prefers the live agent summary over the folded event", () => {
    renderFolded(true);

    expect(screen.getByTestId("background-agent-inline")).toHaveAttribute(
      "data-status",
      "running",
    );
  });
});

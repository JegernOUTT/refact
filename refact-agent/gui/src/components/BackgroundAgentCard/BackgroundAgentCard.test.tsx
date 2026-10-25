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

describe("BackgroundAgentCard", () => {
  it("preserves explicitly expanded panel defaults without opting into inline", () => {
    render(
      <BackgroundAgentCard
        presentation="panel"
        compactDefault={false}
        agent={makeAgent()}
      />,
    );
    expect(screen.getByTestId("background-agent-card")).toHaveAttribute(
      "data-expanded",
      "true",
    );
    expect(screen.getByTestId("background-agent-kind-delegate")).toBeVisible();
    expect(
      screen.getByTestId("background-agent-expanded-detail"),
    ).toBeVisible();
    expect(
      screen.queryByTestId("background-agent-inline"),
    ).not.toBeInTheDocument();
  });

  it("renders the compact row by default with live status, model, tool, usage, questions, and merge", () => {
    render(<BackgroundAgentCard agent={makeAgent()} />);

    expect(screen.getByTestId("background-agent-compact-row")).toBeVisible();
    expect(screen.getByTestId("background-agent-kind-delegate")).toBeVisible();
    expect(screen.getByTestId("background-agent-status-dot")).toHaveAttribute(
      "aria-label",
      "Background agent status: Running",
    );
    expect(screen.getByTestId("background-agent-model")).toHaveTextContent(
      "thinking",
    );
    expect(screen.getByTestId("background-agent-model")).toHaveAttribute(
      "title",
      "openai/gpt-5.6-terra",
    );
    expect(
      screen.getByTestId("background-agent-current-tool"),
    ).toHaveTextContent("now: shell: cargo test --lib background_agent");
    expect(screen.getByTestId("background-agent-usage")).toHaveTextContent(
      "12.3k tok · $0.04",
    );
    expect(screen.getByTestId("background-agent-questions")).toHaveTextContent(
      "❓1",
    );
    expect(screen.getByTestId("background-agent-merge")).toHaveTextContent(
      "Pending",
    );
    expect(
      screen.queryByTestId("background-agent-expanded-detail"),
    ).not.toBeInTheDocument();
  });

  it("always starts collapsed even for running agents", () => {
    render(<BackgroundAgentCard agent={makeAgent()} />);

    expect(
      screen.getByRole("button", { name: "Expand background agent details" }),
    ).toBeVisible();
    expect(
      screen.queryByTestId("background-agent-expanded-detail"),
    ).not.toBeInTheDocument();
  });

  it("expands to show goal, plan, branch, questions, files, activity, and trajectory", () => {
    const onOpenTrajectory = vi.fn();
    render(
      <BackgroundAgentCard
        agent={makeAgent()}
        onOpenTrajectory={onOpenTrajectory}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", { name: "Expand background agent details" }),
    );

    expect(
      screen.getByTestId("background-agent-expanded-detail"),
    ).toBeVisible();
    expect(screen.getByText("🎯 Goal")).toHaveAttribute(
      "title",
      "Ship the compact expandable background-agent card",
    );
    expect(screen.getByText("📋 Plan")).toBeVisible();
    expect(screen.getByText("refact/task/T-8/card")).toBeVisible();
    expect(screen.getByText("Q&A")).toBeVisible();
    expect(
      screen.getByText("Q: Should the card start expanded?"),
    ).toBeVisible();
    expect(screen.getByText("Awaiting reply")).toBeVisible();
    expect(screen.getByText("A: Yes, once the GUI checks pass.")).toBeVisible();
    expect(
      screen.getByText("now: shell: cargo test --lib background_agent"),
    ).toBeVisible();
    expect(screen.getByText("2m ago")).toBeVisible();

    const files = screen.getByRole("button", { name: "2 target files" });
    fireEvent.click(files);
    expect(screen.getByText("BackgroundAgentCard.tsx")).toBeVisible();
    expect(screen.getByText("BackgroundAgentCard.module.css")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Open trajectory" }));
    expect(onOpenTrajectory).toHaveBeenCalledWith("child-chat");
  });

  it("hides the trajectory button without a child chat", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({ child_chat_id: null })}
        onOpenTrajectory={vi.fn()}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", { name: "Expand background agent details" }),
    );

    expect(
      screen.queryByRole("button", { name: "Open trajectory" }),
    ).not.toBeInTheDocument();
  });

  it("shows the running step indicator once in expanded details", () => {
    render(<BackgroundAgentCard agent={makeAgent()} />);

    fireEvent.click(
      screen.getByRole("button", { name: "Expand background agent details" }),
    );

    expect(screen.getByTestId("background-agent-progress")).toBeVisible();
    expect(screen.getAllByText(/step 19/i)).toHaveLength(1);
    expect(screen.queryByText(/Steps: 19/)).not.toBeInTheDocument();
  });

  it("updates the live ticker when agent activity changes", () => {
    const { rerender } = render(<BackgroundAgentCard agent={makeAgent()} />);

    expect(
      screen.getByTestId("background-agent-current-tool"),
    ).toHaveTextContent("shell: cargo test --lib background_agent");

    rerender(
      <BackgroundAgentCard
        agent={makeAgent({
          current_tool: "shell: npm run test backgroundAgents",
        })}
      />,
    );

    expect(
      screen.getByTestId("background-agent-current-tool"),
    ).toHaveTextContent("shell: npm run test backgroundAgents");
  });

  it("renders legacy delegates with the same compact layout", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({
          model: null,
          model_type: null,
          current_tool: null,
          pending_questions: undefined,
          questions: undefined,
          merge_status: null,
        })}
      />,
    );

    expect(screen.getByTestId("background-agent-kind-delegate")).toBeVisible();
    expect(
      screen.getByText("Redesign the background agent card"),
    ).toBeVisible();
    expect(
      screen.queryByTestId("background-agent-model"),
    ).not.toBeInTheDocument();
  });

  it("renders the subagent kind icon", () => {
    render(<BackgroundAgentCard agent={makeAgent({ kind: "subagent" })} />);

    expect(screen.getByTestId("background-agent-kind-subagent")).toBeVisible();
  });

  it("preserves terminal result details and conflict tooltip after expansion", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({
          status: "completed",
          edited_files: ["src/a.ts", "src/b.ts"],
          diff_summary: "+42 -7 across 2 files",
          conflict_summary: "src/a.ts overlaps with delegate two",
          merge_status: "conflict",
          result_summary: "Redesigned the card and updated the tests.",
        })}
      />,
    );

    expect(screen.getByTestId("background-agent-merge")).toHaveAttribute(
      "title",
      "src/a.ts overlaps with delegate two",
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Expand background agent details" }),
    );
    expect(screen.getByText("2 edited")).toBeVisible();
    expect(screen.getByText("+42 −7")).toBeVisible();
    expect(screen.getByText("Conflicts")).toBeVisible();
    expect(
      screen.getByText("Redesigned the card and updated the tests."),
    ).toBeVisible();
  });

  it("renders the structural deliverable verdict with its reasons", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({
          status: "completed",
          deliverable: {
            required_sections: ["Findings"],
            evidence_required: true,
          },
          deliverable_status: {
            check: "structural",
            met: false,
            missing_sections: ["Findings"],
            schema_error: null,
            evidence_missing: true,
          },
        })}
      />,
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Expand background agent details" }),
    );
    const badge = screen.getByTestId("background-agent-deliverable");
    expect(badge).toHaveTextContent("Deliverable unmet");
    expect(badge).toHaveAttribute(
      "title",
      "structural check · missing sections: Findings · evidence section missing; claims are unverified",
    );
  });

  it("distinguishes a declared but unevaluated deliverable contract", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({
          status: "failed",
          error: "provider unavailable",
          deliverable: {
            required_sections: ["Findings"],
            evidence_required: false,
          },
        })}
      />,
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Expand background agent details" }),
    );
    expect(
      screen.getByTestId("background-agent-deliverable"),
    ).toHaveTextContent("Deliverable not evaluated");
  });

  it("hides live tool state and zero cost for terminal agents", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({ status: "completed", cost_usd: 0 })}
      />,
    );

    expect(
      screen.queryByTestId("background-agent-current-tool"),
    ).not.toBeInTheDocument();
    expect(screen.getByTestId("background-agent-usage")).toHaveTextContent(
      "12.3k tok",
    );
    expect(screen.getByTestId("background-agent-usage")).not.toHaveTextContent(
      "$",
    );
    expect(
      screen.getByTestId("background-agent-status-dot").className,
    ).not.toContain("statusDotPulse");

    fireEvent.click(
      screen.getByRole("button", { name: "Expand background agent details" }),
    );
    expect(screen.queryByText(/^now:/)).not.toBeInTheDocument();
  });

  it("never renders raw timestamps and copies the full agent id from the expanded detail", () => {
    const { container } = render(<BackgroundAgentCard agent={makeAgent()} />);

    expect(container.textContent).not.toContain(ISO_TIMESTAMP);
    fireEvent.click(
      screen.getByRole("button", { name: "Expand background agent details" }),
    );
    const chip = screen.getByRole("button", { name: "Copy agent id" });
    expect(chip).toHaveTextContent("3344abcd");
    expect(chip).toHaveAttribute("title", AGENT_ID);

    fireEvent.click(chip);
    expect(writeText).toHaveBeenCalledWith(AGENT_ID);
  });
});

describe("inline background agents", () => {
  it.each([
    ["running", "Running"],
    ["queued", "Queued"],
    ["waiting_for_approval", "Approval needed"],
    ["failed", "Failed"],
    ["completed", "Completed"],
    ["interrupted", "Interrupted"],
    ["cancelled", "Cancelled"],
  ] as const)("starts closed for authoritative %s status", (status, label) => {
    const { container } = render(
      <BackgroundAgentCard
        presentation="inline"
        compactDefault={false}
        agent={makeAgent({
          status,
          title: "Subagent: Inspect the animation",
          error: status === "failed" ? "provider unavailable" : null,
        })}
      />,
    );
    const row = screen.getByTestId("background-agent-inline");
    const toggle = row.querySelector("button");
    expect(row).toHaveAttribute("data-density", "compact");
    expect(row).toHaveAttribute("data-open", "false");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(toggle).toHaveTextContent("Inspect the animation");
    expect(toggle).toHaveTextContent(label);
    expect(screen.getByLabelText("1 pending question")).toBeInTheDocument();
    expect(toggle).not.toHaveTextContent(
      /subagent|thinking|12.3k|\$0.04|pending/i,
    );
    expect(toggle).not.toHaveTextContent(
      /verified|pushed|provider unavailable/i,
    );
    expect(toggle?.querySelector("button, a, input")).toBeNull();
    expect(
      screen.queryByTestId("background-agent-card"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByTestId("background-agent-kind-delegate"),
    ).not.toBeInTheDocument();
    expect(container.querySelector(".rf-expand-grid")).toHaveAttribute("inert");
  });

  it("uses neutral unknown for a launch placeholder rather than synthetic queued", () => {
    const { container } = render(
      <BackgroundAgentCard
        presentation="inline"
        statusKnown={false}
        agent={makeAgent({ status: "queued", pending_questions: 0 })}
      />,
    );
    const toggle = container.querySelector("button");
    expect(toggle).toHaveTextContent("Status unknown");
    expect(toggle).not.toHaveTextContent(/queued|running|completed|verified/i);
    expect(
      screen.getByTestId("background-agent-inline-status"),
    ).toHaveAttribute("data-tone", "muted");
  });

  it("does not mint paused status from arbitrary progress prose", () => {
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
    const row = screen.getByTestId("background-agent-inline");
    expect(row.querySelector("button")).toHaveTextContent("Status unknown");
    expect(row.querySelector("button")).not.toHaveTextContent(
      /paused|completed|verified|pushed/i,
    );
  });

  it("does not upgrade an unrecognized live status over a neutral placeholder", () => {
    const chat = createDefaultChatState();
    const threadId = chat.current_thread_id;
    const agent = makeAgent();
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
    expect(
      screen.getByTestId("background-agent-inline-status"),
    ).toHaveTextContent("Status unknown");
  });

  it("keeps optional tool-call context available in the existing tooltip", () => {
    vi.useFakeTimers();
    try {
      const call: ToolCall = {
        id: "inline-tooltip",
        index: 0,
        function: {
          name: "subagent",
          arguments: '{"task":"Inspect animation","worktree":"isolated"}',
        },
      };
      const { container } = render(
        <BackgroundAgentCard
          presentation="inline"
          toolCall={call}
          agent={makeAgent()}
        />,
      );
      const wrapper = container.querySelector(
        "[data-testid='background-agent-inline']",
      )?.parentElement;
      if (!wrapper) throw new Error("Missing tooltip wrapper");
      fireEvent.mouseEnter(wrapper);
      act(() => {
        vi.advanceTimersByTime(10_000);
      });
      expect(screen.getByText("worktree")).toBeInTheDocument();
      expect(screen.getByText("isolated")).toBeInTheDocument();
      fireEvent.mouseLeave(wrapper);
      act(() => {
        vi.advanceTimersByTime(100);
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it("retains complete details, files and handoff deeper and one trajectory action", async () => {
    const onOpenAgentTrajectory = vi.fn();
    const agent = makeAgent({
      status: "completed",
      merge_status: "merged",
      edited_files: ["src/a.ts", "src/b.ts"],
      result_summary: "Updated animation and tests",
      deliverable_status: {
        check: "structural",
        met: true,
        missing_sections: [],
        evidence_missing: false,
      },
      handoff: {
        sufficiency: "gaps_reported",
        source: "child_declaration",
        claimed_by: AGENT_ID,
        claimed_at_ms: 1,
        gaps: [{ category: "missing_file", detail: "Found a hidden fixture" }],
        notes: [],
      },
    });
    const { container, user } = render(
      <BackgroundAgentCard
        presentation="inline"
        agent={agent}
        onOpenAgentTrajectory={onOpenAgentTrajectory}
      >
        <pre>Original launch payload</pre>
      </BackgroundAgentCard>,
    );
    const row = screen.getByTestId("background-agent-inline");
    const toggle = row.querySelector("button");
    if (!toggle) throw new Error("Missing inline toggle");
    toggle.focus();
    await user.keyboard("{Enter}");
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(container.querySelectorAll("dt")).toHaveLength(3);
    const more = screen.getByRole("button", { name: "More details" });
    expect(more).toHaveAttribute("aria-expanded", "false");
    expect(
      screen.getByRole("button", { name: "Copy agent id" }),
    ).toHaveAttribute("tabindex", "-1");
    const navigate = screen.getByRole("button", { name: "Open trajectory" });
    expect(navigate.closest("button")).toBe(navigate);
    await user.click(navigate);
    expect(onOpenAgentTrajectory).toHaveBeenCalledWith(agent, "child-chat");
    expect(
      screen.getAllByRole("button", { name: "Open trajectory" }),
    ).toHaveLength(1);
    await user.click(more);
    expect(more).toHaveAttribute("aria-expanded", "true");
    expect(
      screen.getByRole("button", { name: "Copy agent id" }),
    ).not.toHaveAttribute("tabindex", "-1");
    expect(screen.getByText("openai/gpt-5.6-terra")).toBeInTheDocument();
    expect(screen.getByText("thinking")).toBeInTheDocument();
    expect(screen.getByText("Local merge: Merged")).toBeInTheDocument();
    expect(screen.getByText("12.3k tok · $0.04")).toBeInTheDocument();
    expect(screen.getByText("refact/task/T-8/card")).toBeInTheDocument();
    expect(screen.getByText("📋 Plan")).toBeInTheDocument();
    expect(
      screen.getByTestId("background-agent-deliverable"),
    ).toHaveTextContent("Deliverable met");
    expect(screen.getByRole("region", { name: "Handoff" })).toHaveTextContent(
      "Found a hidden fixture",
    );
    expect(
      screen.getByText("Q: Should the card start expanded?"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("A: Yes, once the GUI checks pass."),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "2 edited files" }));
    expect(screen.getByText("a.ts")).toHaveAttribute("title", "src/a.ts");
    expect(screen.getByText("b.ts")).toHaveAttribute("title", "src/b.ts");
    expect(screen.getByText("Original launch payload")).toBeInTheDocument();
    expect(toggle).not.toHaveTextContent(/verified|pushed|merged/i);
    await user.click(toggle);
    expect(container.querySelector(".rf-expand-grid")).toHaveAttribute("inert");
    expect(navigate).toHaveAttribute("tabindex", "-1");
  });

  it.each([null, "child-chat"])(
    "omits unavailable navigation for child %s",
    (childChatId) => {
      render(
        <BackgroundAgentCard
          presentation="inline"
          agent={makeAgent({ child_chat_id: childChatId })}
          onOpenTrajectory={childChatId === null ? vi.fn() : undefined}
        />,
      );
      expect(
        screen.queryByRole("button", { name: "Open trajectory" }),
      ).not.toBeInTheDocument();
    },
  );

  it("uses the existing live subscription and does not reopen on sequence or status updates", () => {
    const chat = createDefaultChatState();
    const threadId = chat.current_thread_id;
    const agent = makeAgent({ status: "queued", pending_questions: 0 });
    const { store, container } = render(
      <BackgroundAgentCard
        presentation="inline"
        agent={agent}
        threadId={threadId}
        statusKnown={false}
      />,
      { preloadedState: { chat } },
    );
    const row = screen.getByTestId("background-agent-inline");
    const toggle = row.querySelector("button");
    expect(toggle).toHaveTextContent("Status unknown");
    const update = (
      seq: number,
      overrides: Partial<BackgroundAgentSummary>,
    ) => {
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
    };
    update(1, { status: "running", title: "Live title" });
    expect(toggle).toHaveTextContent("Live titleRunning");
    expect(row).toHaveAttribute("data-open", "false");
    if (!toggle) throw new Error("Missing inline toggle");
    fireEvent.click(toggle);
    update(2, { status: "waiting_for_approval", pending_questions: 2 });
    expect(row).toHaveAttribute("data-open", "true");
    expect(toggle).toHaveTextContent("Approval needed2");
    expect(screen.getByLabelText("2 pending questions")).toBeInTheDocument();
    fireEvent.click(toggle);
    update(3, { status: "failed", error: "new failure" });
    expect(toggle).toHaveTextContent("Failed");
    expect(row).toHaveAttribute("data-open", "false");
    expect(container.querySelector(".rf-expand-grid")).toHaveAttribute("inert");
    update(2, { status: "running" });
    expect(toggle).toHaveTextContent("Failed");
  });

  it.each([true, false])(
    "retains an explicit %s choice across remounts without affecting another row",
    (open) => {
      const values = new Map<string, boolean>([["tc:launch-one", open]]);
      const collapseStore = {
        get: (key: string) => values.get(key),
        set: (key: string, value: boolean) => values.set(key, value),
      };
      const view = (
        <CollapsibleStoreProvider value={collapseStore}>
          <BackgroundAgentCard
            presentation="inline"
            toolCallId="launch-one"
            agent={makeAgent()}
          />
          <BackgroundAgentCard
            presentation="inline"
            toolCallId="launch-two"
            agent={makeAgent({ agent_id: "second" })}
          />
        </CollapsibleStoreProvider>
      );
      const first = render(view);
      const rows = screen.getAllByTestId("background-agent-inline");
      expect(rows[0]).toHaveAttribute("data-open", String(open));
      expect(rows[1]).toHaveAttribute("data-open", "false");
      const toggle = rows[0].querySelector("button");
      if (!toggle) throw new Error("Missing inline toggle");
      fireEvent.click(toggle);
      first.unmount();
      render(view);
      const remounted = screen.getAllByTestId("background-agent-inline");
      expect(remounted[0]).toHaveAttribute("data-open", String(!open));
      expect(remounted[1]).toHaveAttribute("data-open", "false");
    },
  );

  it("anchors inline expansion through the existing scroll context", () => {
    const preserve = vi.fn((mutate: () => void) => mutate());
    const prepare = vi.fn();
    render(
      <ChatScrollAnchorContext.Provider
        value={{ preserveScrollAnchor: preserve, prepareScrollAnchor: prepare }}
      >
        <BackgroundAgentCard presentation="inline" agent={makeAgent()} />
      </ChatScrollAnchorContext.Provider>,
    );
    const toggle = screen
      .getByTestId("background-agent-inline")
      .querySelector("button");
    if (!toggle) throw new Error("Missing inline toggle");
    fireEvent.keyDown(toggle, { key: "Enter" });
    fireEvent.click(toggle);
    expect(prepare).toHaveBeenCalled();
    expect(preserve).toHaveBeenCalledOnce();
  });

  it("bounds and escapes a long hostile title without losing essential status", () => {
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
    const toggle = row.querySelector("button");
    expect(toggle).toHaveTextContent("<script>attack()</script>");
    expect(toggle).toHaveTextContent("…Running");
  });
});

describe("legacy SubagentTool inline fallback", () => {
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
    "# Subagent Result\n\n**Task:** Inspect animation\n\n## Response\n\nAnimation findings preserved.";

  function renderLegacy(
    content?: string,
    failed = false,
    argumentsValue = call.function.arguments,
    presentation?: "panel" | "inline",
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
          presentation={presentation ?? "inline"}
          toolCall={{
            ...call,
            function: { ...call.function, arguments: argumentsValue },
          }}
        />
      </ChatThreadProvider>,
      { preloadedState: { chat } },
    );
  }

  it("retains the noncompact report default when the legacy caller does not opt in", () => {
    const { container } = renderLegacy(
      legacy,
      false,
      call.function.arguments,
      "panel",
    );
    expect(container.querySelector("section")).toHaveAttribute(
      "data-open",
      "true",
    );
    expect(container.querySelector("section")).not.toHaveAttribute(
      "data-density",
      "compact",
    );
    expect(
      screen.getByText("Animation findings preserved."),
    ).toBeInTheDocument();
  });

  it("removes boilerplate, preserves full report and raw launch payload deeper", async () => {
    const { container, user } = renderLegacy(legacy);
    const toggle = screen.getByRole("button");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(toggle).toHaveTextContent("Inspect animation");
    expect(toggle).toHaveTextContent("Report unverified");
    expect(toggle).not.toHaveTextContent(/Subagent:|Analyze|model|files/i);
    await user.click(toggle);
    expect(container.querySelectorAll("dt").length).toBeLessThanOrEqual(3);
    await user.click(screen.getByRole("button", { name: "More details" }));
    expect(
      screen.getByText("Animation findings preserved."),
    ).toBeInTheDocument();
    expect(
      screen.getByText("model: private-model · files: 1"),
    ).toBeInTheDocument();
    const raw = screen.getByRole("button", { name: "Technical details" });
    await user.click(raw);
    const region = document.getElementById(
      raw.getAttribute("aria-controls") ?? "",
    );
    if (!region) throw new Error("Missing raw payload");
    expect(region).toHaveTextContent("# Subagent Result");
    expect(region).toHaveTextContent("**Task:** Inspect animation");
    expect(region).toHaveTextContent('"target_files"');
    expect(
      screen.getByRole("button", { name: "Copy report" }),
    ).toBeInTheDocument();
  });

  it.each([
    [undefined, false, "Awaiting report"],
    ["queued is only prose", false, "Report unverified"],
    ["Launch failed", true, "Request failed"],
  ] as const)(
    "does not infer child lifecycle from %s",
    (content, failed, label) => {
      renderLegacy(content, failed);
      const toggle = screen.getByRole("button");
      expect(toggle).toHaveAttribute("aria-expanded", "false");
      expect(toggle).toHaveTextContent(label);
      expect(toggle).not.toHaveTextContent(
        /queued|completed|verified and|pushed/i,
      );
    },
  );

  it.each(["null", "[]", "broken", '{"task": {"text": "not scalar"}}'])(
    "handles malformed legacy arguments %s",
    (args) => {
      renderLegacy(undefined, false, args);
      expect(screen.getByRole("button")).toHaveTextContent("Background work");
    },
  );
});

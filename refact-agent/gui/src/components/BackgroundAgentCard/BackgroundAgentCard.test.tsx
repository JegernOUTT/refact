import { describe, expect, it, vi, beforeEach } from "vitest";
import { fireEvent, render, screen } from "../../utils/test-utils";
import { BackgroundAgentCard } from "./BackgroundAgentCard";
import type { BackgroundAgentSummary } from "../../services/refact/types";

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
    expect(screen.getByTestId("background-agent-status-dot").className).not.toContain(
      "statusDotPulse",
    );

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

import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "../../utils/test-utils";
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
    progress: null,
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
  it("renders a delegate kind tile", () => {
    render(<BackgroundAgentCard agent={makeAgent()} />);

    expect(screen.getByTestId("background-agent-kind-delegate")).toBeVisible();
    expect(
      screen.queryByTestId("background-agent-kind-subagent"),
    ).not.toBeInTheDocument();
  });

  it("renders a subagent kind tile", () => {
    render(<BackgroundAgentCard agent={makeAgent({ kind: "subagent" })} />);

    expect(screen.getByTestId("background-agent-kind-subagent")).toBeVisible();
  });

  it("humanizes the status chip instead of showing the raw enum", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({ status: "waiting_for_approval" })}
      />,
    );

    const chip = screen.getByTestId("background-agent-status");
    expect(chip).toHaveTextContent("Waiting for approval");
    expect(chip.textContent).not.toContain("waiting_for_approval");
  });

  it("never renders a raw ISO timestamp and shows relative activity instead", () => {
    const { container } = render(<BackgroundAgentCard agent={makeAgent()} />);

    expect(container.textContent).not.toContain(ISO_TIMESTAMP);
    expect(container.textContent).not.toMatch(/\d{4}-\d{2}-\d{2}T/);
    expect(screen.getByText("2m ago")).toBeVisible();
  });

  it("shows a single running step indicator without duplicating step counts", () => {
    render(<BackgroundAgentCard agent={makeAgent()} />);

    expect(screen.getByTestId("background-agent-progress")).toBeVisible();
    expect(screen.getByText(/step 19/)).toBeVisible();
    expect(screen.queryByText(/Steps: 19/)).not.toBeInTheDocument();
  });

  it("collapses files into a chip that expands to shortened paths with one prefix line", () => {
    render(<BackgroundAgentCard agent={makeAgent()} />);

    const toggle = screen.getByRole("button", { name: "2 target files" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(
      screen.queryByText("BackgroundAgentCard.tsx"),
    ).not.toBeInTheDocument();

    fireEvent.click(toggle);

    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("BackgroundAgentCard.tsx")).toBeVisible();
    expect(screen.getByText("BackgroundAgentCard.module.css")).toBeVisible();
    expect(
      screen.getByText(
        "…in refact-agent/gui/src/components/BackgroundAgentCard/",
      ),
    ).toBeVisible();
  });

  it("prefers edited files on terminal states", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({
          status: "completed",
          edited_files: ["src/a.ts"],
        })}
      />,
    );

    expect(
      screen.getByRole("button", { name: "1 edited files" }),
    ).toBeInTheDocument();
  });

  it("copies the full agent id from the short-id chip", () => {
    render(<BackgroundAgentCard agent={makeAgent()} />);

    const chip = screen.getByRole("button", { name: "Copy agent id" });
    expect(chip).toHaveTextContent("3344abcd");
    expect(chip).toHaveAttribute("title", AGENT_ID);
    expect(chip.textContent).not.toContain(AGENT_ID);

    fireEvent.click(chip);

    expect(writeText).toHaveBeenCalledWith(AGENT_ID);
  });

  it("renders a compact result line with chips when terminal", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({
          status: "completed",
          last_activity: null,
          edited_files: ["src/a.ts", "src/b.ts"],
          diff_summary: "+42 -7 across 2 files",
          conflict_summary: "src/a.ts overlaps with delegate two",
          result_summary: "Redesigned the card and updated the tests.",
        })}
      />,
    );

    expect(
      screen.queryByTestId("background-agent-progress"),
    ).not.toBeInTheDocument();
    expect(screen.getByText("2 edited")).toBeVisible();
    expect(screen.getByText("+42 −7")).toBeVisible();
    expect(screen.getByText("Conflicts")).toBeVisible();
    expect(
      screen.getByText("Redesigned the card and updated the tests."),
    ).toBeVisible();
  });

  it("opens the child trajectory from the footer button", () => {
    const onOpenTrajectory = vi.fn();
    render(
      <BackgroundAgentCard
        agent={makeAgent()}
        onOpenTrajectory={onOpenTrajectory}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Open trajectory" }));

    expect(onOpenTrajectory).toHaveBeenCalledWith("child-chat");
  });

  it("hides the trajectory button when there is no child chat", () => {
    render(<BackgroundAgentCard agent={makeAgent({ child_chat_id: null })} />);

    expect(
      screen.queryByRole("button", { name: "Open trajectory" }),
    ).not.toBeInTheDocument();
  });
});

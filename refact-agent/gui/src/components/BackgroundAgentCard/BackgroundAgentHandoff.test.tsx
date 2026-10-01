import { fireEvent, render, screen, within } from "../../utils/test-utils";
import { describe, expect, it } from "vitest";
import { BackgroundAgentCard } from "./BackgroundAgentCard";
import type { BackgroundAgentSummary } from "../../services/refact/types";
import {
  normalizeBackgroundAgentSummary,
  type BackgroundAgentSummaryWire,
} from "../../services/refact/chatSubscription";

function makeAgent(
  overrides: Partial<BackgroundAgentSummary> = {},
): BackgroundAgentSummary {
  return {
    agent_id: "agent-1",
    parent_chat_id: "parent-chat",
    child_chat_id: "child-chat",
    kind: "subagent",
    status: "completed",
    title: "Inspect handoff gaps",
    progress: null,
    step_count: 1,
    last_activity: null,
    target_files: [],
    edited_files: [],
    diff_summary: null,
    conflict_summary: null,
    result_summary: "Finished inspection",
    error: null,
    started_at: null,
    finished_at: null,
    change_seq: 1,
    ...overrides,
  };
}

function gap(category: "missing_file" | "missing_decision", detail: string) {
  return { category, detail };
}

describe("BackgroundAgentCard handoff", () => {
  it("renders discovered and guessed items for a completed agent", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({
          handoff: {
            sufficiency: "gaps_reported",
            source: "child_declaration",
            claimed_by: "agent-1",
            claimed_at_ms: 1,
            gaps: [
              {
                ...gap("missing_file", "Read the retry policy"),
                path: "src/retry.ts",
              },
              gap("missing_decision", "Choose the fallback policy"),
            ],
            notes: [],
          },
        })}
      />,
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Expand background agent details" }),
    );

    const handoff = screen.getByRole("region", { name: "Handoff" });
    expect(
      within(handoff).getByRole("heading", { name: "Handoff" }),
    ).toBeVisible();
    expect(
      within(handoff).getByRole("heading", { name: "Had to discover" }),
    ).toBeVisible();
    expect(
      within(handoff).getByText("Read the retry policy (src/retry.ts)"),
    ).toBeVisible();
    expect(
      within(handoff).getByRole("heading", { name: "Had to guess" }),
    ).toBeVisible();
    expect(
      within(handoff).getByText("Choose the fallback policy"),
    ).toBeVisible();
  });

  it.each([undefined, []])(
    "renders Nothing declared for an empty or absent declaration on a completed agent",
    (gaps) => {
      render(
        <BackgroundAgentCard
          agent={makeAgent({
            handoff:
              gaps === undefined
                ? undefined
                : {
                    sufficiency: "sufficient",
                    source: "child_declaration",
                    claimed_by: "agent-1",
                    claimed_at_ms: 1,
                    gaps,
                    notes: [],
                  },
          })}
        />,
      );
      fireEvent.click(
        screen.getByRole("button", { name: "Expand background agent details" }),
      );
      expect(screen.getByRole("region", { name: "Handoff" })).toHaveTextContent(
        "Nothing declared",
      );
    },
  );

  it("renders no handoff for a running agent", () => {
    render(
      <BackgroundAgentCard
        agent={makeAgent({
          status: "running",
          handoff: {
            sufficiency: "gaps_reported",
            source: "child_declaration",
            claimed_by: "agent-1",
            claimed_at_ms: 1,
            gaps: [gap("missing_file", "Read the retry policy")],
            notes: [],
          },
        })}
      />,
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Expand background agent details" }),
    );
    expect(
      screen.queryByRole("region", { name: "Handoff" }),
    ).not.toBeInTheDocument();
  });

  it("drops malformed handoff data during subscription normalization without throwing", () => {
    const malformed = {
      agent_id: "agent-1",
      parent_chat_id: "parent-chat",
      child_chat_id: "child-chat",
      kind: "subagent",
      status: "completed",
      handoff: {
        sufficiency: "not-a-status",
        source: [],
        claimed_by: "agent-1",
        claimed_at_ms: "later",
        gaps: [{ category: "missing_file", detail: 17 }],
        notes: [],
      },
    } as unknown as BackgroundAgentSummaryWire;
    expect(() => normalizeBackgroundAgentSummary(malformed)).not.toThrow();
    const normalized = normalizeBackgroundAgentSummary(malformed);
    expect(normalized.handoff).toBeUndefined();
  });

  it("normalizes the engine wire shape without notes or optional gap fields", () => {
    const wireAgent = {
      agent_id: "agent-1",
      parent_chat_id: "parent-chat",
      child_chat_id: "child-chat",
      kind: "subagent",
      status: "completed",
      title: "Inspect handoff gaps",
      progress: null,
      step_count: 1,
      last_activity: null,
      target_files: [],
      edited_files: [],
      diff_summary: null,
      conflict_summary: null,
      result_summary: "Finished inspection",
      error: null,
      started_at: null,
      finished_at: null,
      change_seq: 1,
      handoff: {
        sufficiency: "gaps_reported",
        source: "child_declaration",
        claimed_by: "agent-1",
        claimed_at_ms: 1,
        gaps: [
          { category: "missing_file", detail: "Read the retry policy" },
          {
            category: "missing_decision",
            detail: "Choose the fallback policy",
            path: "src/retry.ts",
          },
        ],
      },
    } as unknown as BackgroundAgentSummaryWire;
    const normalized = normalizeBackgroundAgentSummary(wireAgent);

    expect(normalized.handoff).toBeDefined();
    expect(normalized.handoff?.gaps).toHaveLength(2);
    expect(normalized.handoff?.notes).toEqual([]);
    render(<BackgroundAgentCard agent={normalized} />);
    fireEvent.click(
      screen.getByRole("button", { name: "Expand background agent details" }),
    );

    const handoff = screen.getByRole("region", { name: "Handoff" });
    expect(
      within(handoff).getByRole("heading", { name: "Had to discover" }),
    ).toBeVisible();
    expect(within(handoff).getByText("Read the retry policy")).toBeVisible();
    expect(
      within(handoff).getByRole("heading", { name: "Had to guess" }),
    ).toBeVisible();
    expect(
      within(handoff).getByText("Choose the fallback policy (src/retry.ts)"),
    ).toBeVisible();
  });

  it("drops handoff data when notes are present but malformed", () => {
    const malformed = {
      agent_id: "agent-1",
      parent_chat_id: "parent-chat",
      child_chat_id: "child-chat",
      kind: "subagent",
      status: "completed",
      handoff: {
        sufficiency: "sufficient",
        source: "child_declaration",
        claimed_by: "agent-1",
        claimed_at_ms: 1,
        gaps: [],
        notes: [17],
      },
    } as unknown as BackgroundAgentSummaryWire;

    expect(() => normalizeBackgroundAgentSummary(malformed)).not.toThrow();
    expect(normalizeBackgroundAgentSummary(malformed).handoff).toBeUndefined();
  });

  it("limits each list to ten items and bounds rendered item text", () => {
    const details = Array.from(
      { length: 12 },
      (_, index) => `Discovery ${index + 1}`,
    );
    render(
      <BackgroundAgentCard
        agent={makeAgent({
          handoff: {
            sufficiency: "gaps_reported",
            source: "child_declaration",
            claimed_by: "agent-1",
            claimed_at_ms: 1,
            gaps: [
              ...details
                .slice(0, 9)
                .map((detail) => gap("missing_file", detail)),
              gap("missing_file", "x".repeat(400)),
              ...details.slice(9).map((detail) => gap("missing_file", detail)),
            ],
            notes: [],
          },
        })}
      />,
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Expand background agent details" }),
    );

    const handoff = screen.getByRole("region", { name: "Handoff" });
    expect(within(handoff).getAllByRole("listitem")).toHaveLength(10);
    expect(within(handoff).getByText("+3 more")).toBeVisible();
    expect(within(handoff).getByText("Discovery 9")).toBeVisible();
    expect(within(handoff).queryByText("Discovery 10")).not.toBeInTheDocument();
    const lastItem = within(handoff).getAllByRole("listitem")[9];
    expect(lastItem.textContent).toHaveLength(300);
  });
});

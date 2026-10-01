import { describe, expect, it } from "vitest";
import { render, screen } from "../../utils/test-utils";
import type {
  BoardCard,
  CardOutcome,
  TaskBoard,
} from "../../services/refact/tasks";
import {
  normalizeCardOutcomes,
  normalizeTaskBoard,
} from "../../services/refact/tasks";
import { KanbanBoard } from "./KanbanBoard";
import { CardOutcomeBadge } from "./CardOutcomeBadge";

const card: BoardCard = {
  id: "T-1",
  title: "Task card",
  column: "planned",
  priority: "P1",
  depends_on: [],
  instructions: "",
  assignee: null,
  agent_chat_id: null,
  status_updates: [],
  final_report: null,
  created_at: "2026-10-01T00:00:00Z",
  started_at: null,
  completed_at: null,
  target_files: [],
};

const partialOutcome: CardOutcome = {
  required: [
    { fragment: "summary", requires_evidence: false },
    { fragment: "verification", requires_evidence: false },
  ],
  fragments: [
    {
      name: "summary",
      content: "Implemented the change.",
      provenance: { source_run: "agent-1", evidence_spec_ids: [] },
      review: "pending",
      reviewer_chat_id: null,
      reviewed_at: null,
      review_note: null,
    },
  ],
  finalized: null,
};

function makeBoard(cardOutcomes?: Record<string, CardOutcome>): TaskBoard {
  return {
    schema_version: 1,
    rev: 1,
    columns: [{ id: "planned", title: "Planned" }],
    cards: [card],
    card_outcomes: cardOutcomes,
  };
}

describe("CardOutcomeBadge", () => {
  it("shows finalized status with finalizer and timestamp in its tooltip", async () => {
    const finalizedOutcome: CardOutcome = {
      ...partialOutcome,
      finalized: {
        finalized_by: "reviewer-chat",
        finalized_at: "2026-10-01T12:30:00Z",
      },
    };
    const { user } = render(
      <CardOutcomeBadge
        cardId="T-1"
        cardOutcomes={{ "T-1": finalizedOutcome }}
      />,
    );

    const badge = screen.getByRole("status", {
      name: "Finalized by reviewer-chat at 2026-10-01T12:30:00Z",
    });
    expect(badge).toHaveTextContent("Finalized");
    await user.hover(badge);
    expect(await screen.findByRole("tooltip")).toHaveTextContent(
      "Finalized by reviewer-chat at 2026-10-01T12:30:00Z",
    );
  });

  it("counts recorded required fragments", () => {
    render(
      <CardOutcomeBadge
        cardId="T-1"
        cardOutcomes={{ "T-1": partialOutcome }}
      />,
    );

    expect(
      screen.getByRole("status", { name: "Outcome: 1/2 fragments" }),
    ).toHaveTextContent("Outcome: 1/2 fragments");
  });

  it("renders nothing when the card has no outcome entry", () => {
    render(<CardOutcomeBadge cardId="T-1" />);

    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("drops malformed outcome entries without throwing", () => {
    const input = {
      "T-1": partialOutcome,
      "T-2": { required: "not-an-array", fragments: [], finalized: null },
    };

    expect(() => normalizeCardOutcomes(input)).not.toThrow();
    expect(normalizeCardOutcomes(input)).toEqual({ "T-1": partialOutcome });
  });

  it("normalizes malformed outcomes off a task board without throwing", () => {
    const input = {
      ...makeBoard(),
      card_outcomes: {
        "T-1": partialOutcome,
        "T-2": { required: "not-an-array", fragments: [], finalized: null },
      },
    };

    expect(() => normalizeTaskBoard(input)).not.toThrow();
    expect(normalizeTaskBoard(input)?.card_outcomes).toEqual({
      "T-1": partialOutcome,
    });
  });

  it("renders board card outcomes in the Kanban card badge row", () => {
    render(<KanbanBoard board={makeBoard({ "T-1": partialOutcome })} />);

    expect(screen.getByText("Task card")).toBeInTheDocument();
    expect(
      screen.getByRole("status", { name: "Outcome: 1/2 fragments" }),
    ).toBeInTheDocument();
  });
});

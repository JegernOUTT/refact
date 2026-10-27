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
import { CardEvidencePanel } from "./CardEvidencePanel";

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

  it("keeps reported success separate from failed, blocked, not-run and cached verifier evidence", async () => {
    const evidenceCard: BoardCard = {
      ...card,
      column: "done",
      final_report_structured: { success: true, summary: "Agent says done" },
      verifier_report: {
        passed: false,
        classification: "verification_failed",
        concerns: ["Missing required coverage"],
        command_results: [
          {
            command: "npm test",
            passed: false,
            outcome: "command_failed",
            exit_code: 1,
            output_tail: "A failing assertion at the end",
          },
          {
            command: "blocked-command",
            passed: false,
            outcome: "policy_denied",
            exit_code: null,
          },
          { command: "not-run-command", passed: false, outcome: "no_commands" },
          {
            command: "cached-command",
            passed: true,
            outcome: "passed",
            exit_code: 0,
            from_cache: true,
            executed_at: "2026-10-01T12:30:00Z",
          },
          { command: "legacy-command", passed: true, exit_code: 0 },
        ],
      },
    };
    const board = normalizeTaskBoard({
      ...makeBoard({ "T-1": partialOutcome }),
      cards: [evidenceCard],
      owner_chat_id: "actual-owner",
      card_claims: {
        "T-1": {
          claimant_chat_id: "agent-1",
          claimed_at: "t0",
          idempotency_key: "never-show-this-secret",
        },
      },
    });
    if (!board) throw new Error("Invalid evidence fixture");
    const { user } = render(
      <CardEvidencePanel card={board.cards[0]} board={board} />,
    );
    expect(screen.getByText("Verifier: Failed")).toBeInTheDocument();
    expect(screen.getByText("Verifier: Blocked — not run")).toBeInTheDocument();
    expect(screen.getByText("Verifier: Not run")).toBeInTheDocument();
    expect(
      screen.getByText(/Cached — replayed, not re-executed/),
    ).toHaveTextContent("2026-10-01T12:30:00Z");
    expect(
      screen.getByText("Verifier: Unknown — legacy or unclassified evidence"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Spec revision \/ verified HEAD: not included/),
    ).toBeInTheDocument();
    await user.click(
      screen.getByText(
        "Agent-reported structured report (not independently verified)",
      ),
    );
    expect(screen.getByText("Reported success")).toBeInTheDocument();
    await user.click(screen.getByText("Full recorded log tail"));
    expect(
      screen.getByText("A failing assertion at the end"),
    ).toBeInTheDocument();
    await user.click(screen.getByText("Outcome fragments and provenance"));
    expect(screen.getByText("Source run: agent-1")).toBeInTheDocument();
    expect(
      screen.getByText(/Not recorded — not inferred from Done/),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("never-show-this-secret"),
    ).not.toBeInTheDocument();
    expect(JSON.stringify(board.card_claims)).not.toContain(
      "never-show-this-secret",
    );
  });

  it("treats legacy or malformed optional evidence as unknown, never a pass", async () => {
    const board = normalizeTaskBoard({
      ...makeBoard(),
      cards: [
        {
          ...card,
          final_report_structured: { verification: "invalid" },
          verifier_report: { command_results: [{ command: 42, passed: true }] },
        },
      ],
    });
    if (!board) throw new Error("Invalid legacy fixture");
    const { user } = render(
      <CardEvidencePanel card={board.cards[0]} board={board} />,
    );
    await user.click(
      screen.getByText("Verifier report (separate from the agent claim)"),
    );
    expect(
      screen.getByText("No verifier report. Verification unknown; not a pass."),
    ).toBeInTheDocument();
    expect(screen.queryByText("Verifier: Passed")).not.toBeInTheDocument();
    expect(screen.getByText("Unknown — no owner recorded")).toBeInTheDocument();
  });
});

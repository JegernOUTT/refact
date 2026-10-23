import { describe, expect, it } from "vitest";
import { render, screen } from "../../utils/test-utils";
import {
  normalizeCardAttention,
  normalizeTaskBoard,
  type BoardCard,
  type CardAttention,
  type TaskBoard,
} from "../../services/refact/tasks";
import { CardAttentionBadge } from "./CardAttentionBadge";
import { KanbanBoard } from "./KanbanBoard";

const since = "2026-10-02T12:30:00Z";
const attention: CardAttention = {
  reason: "Waiting for the session to reconnect",
  kind: "no_session",
  since,
  count: 3,
};

const card: BoardCard = {
  id: "T-1",
  title: "Recoverable task card",
  column: "doing",
  priority: "P1",
  depends_on: [],
  instructions: "",
  assignee: null,
  agent_chat_id: null,
  status_updates: [],
  final_report: null,
  created_at: "2026-10-01T00:00:00Z",
  started_at: since,
  completed_at: null,
  target_files: [],
};

function makeBoard(): TaskBoard {
  return {
    schema_version: 1,
    rev: 1,
    columns: [{ id: "doing", title: "Doing" }],
    cards: [card],
  };
}

describe("CardAttentionBadge", () => {
  it("renders attention with reason, kind, since and count in its tooltip", async () => {
    const { user } = render(<CardAttentionBadge card={{ attention }} />);

    const badge = screen.getByRole("status", { name: /^Needs attention:/ });
    expect(badge).toHaveTextContent("Needs attention");
    await user.hover(badge);
    expect(await screen.findByRole("tooltip")).toHaveTextContent(
      `Needs attention: ${attention.reason} · Kind: no_session · Since: ${since} · ×3`,
    );
  });

  it.each(["transient", "stall", "no_session", "unknown", "future_kind"])(
    "preserves and displays attention kind %s with a zero count",
    async (kind) => {
      const normalized = normalizeCardAttention({
        ...attention,
        kind,
        count: 0,
      });
      expect(normalized).toEqual({ ...attention, kind, count: 0 });
      const { user } = render(
        <CardAttentionBadge card={{ attention: normalized }} />,
      );

      await user.hover(
        screen.getByRole("status", { name: /^Needs attention:/ }),
      );
      const tooltip = await screen.findByRole("tooltip");
      expect(tooltip).toHaveTextContent(`Kind: ${kind}`);
      expect(tooltip).toHaveTextContent("×0");
    },
  );

  it.each(["planner-chat", undefined])(
    "renders paused with paused_by %s in its tooltip when present",
    async (pausedBy) => {
      const { user } = render(
        <CardAttentionBadge card={{ paused_at: since, paused_by: pausedBy }} />,
      );

      const badge = screen.getByRole("status", { name: /^Paused since/ });
      expect(badge).toHaveTextContent("Paused");
      await user.hover(badge);
      expect(await screen.findByRole("tooltip")).toHaveTextContent(
        `Paused since ${since}${pausedBy ? ` by ${pausedBy}` : ""}`,
      );
      if (!pausedBy) {
        expect(await screen.findByRole("tooltip")).not.toHaveTextContent(
          " by ",
        );
      }
    },
  );

  it("renders nothing when neither attention nor paused_at is present", () => {
    render(<CardAttentionBadge card={{ paused_by: "planner-chat" }} />);

    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    expect(screen.queryByText("Needs attention")).not.toBeInTheDocument();
    expect(screen.queryByText("Paused")).not.toBeInTheDocument();
  });

  it.each([
    { kind: "stall", since, count: 1 },
    { ...attention, reason: 42 },
    { ...attention, kind: undefined },
    { ...attention, kind: 42 },
    { ...attention, since: undefined },
    { ...attention, since: 42 },
    { ...attention, count: -1 },
    { ...attention, count: 1.5 },
    { ...attention, count: "3" },
    { ...attention, count: undefined },
    { ...attention, count: NaN },
    { ...attention, count: Infinity },
    "not-an-object",
    [],
    null,
    undefined,
  ])("drops malformed attention without throwing: %j", (value) => {
    expect(() => normalizeCardAttention(value)).not.toThrow();
    expect(normalizeCardAttention(value)).toBeUndefined();

    const input = { ...makeBoard(), cards: [{ ...card, attention: value }] };
    expect(() => normalizeTaskBoard(input)).not.toThrow();
    expect(normalizeTaskBoard(input)?.cards[0].attention).toBeUndefined();
    expect(input.cards[0].attention).toBe(value);
  });

  it("normalizes valid lifecycle fields without changing their values", () => {
    const lifecycle = {
      attention,
      paused_at: since,
      paused_by: "planner-chat",
    };
    const board = normalizeTaskBoard({
      ...makeBoard(),
      cards: [{ ...card, ...lifecycle }],
    });

    expect(board?.cards[0]).toEqual({ ...card, ...lifecycle });
  });

  it("drops malformed pause fields independently without throwing", () => {
    const input = {
      ...makeBoard(),
      cards: [
        { ...card, paused_at: {}, paused_by: 42 },
        { ...card, paused_at: since, paused_by: null },
        { ...card, paused_at: null, paused_by: "planner-chat" },
      ],
    };

    expect(() => normalizeTaskBoard(input)).not.toThrow();
    expect(normalizeTaskBoard(input)?.cards).toEqual([
      { ...card, paused_at: undefined, paused_by: undefined },
      { ...card, paused_at: since, paused_by: undefined },
      { ...card, paused_at: undefined, paused_by: "planner-chat" },
    ]);
  });

  it("renders both lifecycle badges beside the outcome in the Kanban card badge row", () => {
    const board: TaskBoard = {
      ...makeBoard(),
      cards: [
        { ...card, attention, paused_at: since, paused_by: "planner-chat" },
      ],
      card_outcomes: {
        "T-1": { required: [], fragments: [], finalized: null },
      },
    };
    render(<KanbanBoard board={board} />);

    expect(screen.getByText("Recoverable task card")).toBeInTheDocument();
    const outcome = screen.getByRole("status", {
      name: "Outcome: 0/0 fragments",
    });
    const attentionBadge = screen.getByRole("status", {
      name: /^Needs attention:/,
    });
    const pausedBadge = screen.getByRole("status", {
      name: `Paused since ${since} by planner-chat`,
    });
    expect(attentionBadge).toHaveTextContent("Needs attention");
    expect(pausedBadge).toHaveTextContent("Paused");
    expect(attentionBadge.parentElement).toBe(outcome.parentElement);
    expect(pausedBadge.parentElement).toBe(outcome.parentElement);
  });
});

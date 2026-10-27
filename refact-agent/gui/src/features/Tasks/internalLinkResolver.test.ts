import { describe, it, expect } from "vitest";
import { resolveChatLink } from "./internalLinkResolver";
import type { TaskBoard, BoardCard } from "../../services/refact/tasks";
import type { PlannerInfo } from "./tasksSlice";

function makePlanner(id: string): PlannerInfo {
  const now = new Date().toISOString();
  return {
    id,
    title: `planner ${id}`,
    createdAt: now,
    updatedAt: now,
  };
}

function makeCard(id: string, agentChatId: string | null): BoardCard {
  return {
    id,
    title: `card ${id}`,
    column: "doing",
    priority: "P1",
    depends_on: [],
    created_at: new Date().toISOString(),
    agent_chat_id: agentChatId,
    target_files: [],
  } as unknown as BoardCard;
}

function makeBoard(cards: BoardCard[]): TaskBoard {
  return {
    rev: 1,
    cards,
  } as unknown as TaskBoard;
}

describe("resolveChatLink", () => {
  it("returns planner when chatId matches a planner chat in this task", () => {
    const planners = [makePlanner("planner-abc"), makePlanner("planner-xyz")];
    const board = makeBoard([makeCard("T-1", "agent-T-1-1234abcd")]);

    const result = resolveChatLink("planner-abc", planners, board);

    expect(result).toEqual({ kind: "planner", chatId: "planner-abc" });
  });

  it("prefers planner over agent when both have matching ids (planner wins)", () => {
    const planners = [makePlanner("shared-id")];
    const board = makeBoard([makeCard("T-1", "shared-id")]);

    const result = resolveChatLink("shared-id", planners, board);

    expect(result).toEqual({ kind: "planner", chatId: "shared-id" });
  });

  it("returns agent when chatId matches a card's agent_chat_id", () => {
    const planners = [makePlanner("planner-abc")];
    const board = makeBoard([
      makeCard("T-1", "agent-T-1-1234abcd"),
      makeCard("T-2", "agent-T-2-5678efgh"),
    ]);

    const result = resolveChatLink("agent-T-2-5678efgh", planners, board);

    expect(result).toEqual({
      kind: "agent",
      cardId: "T-2",
      chatId: "agent-T-2-5678efgh",
    });
  });

  it("does not infer card authority from legacy agent ID spelling", () => {
    const planners: PlannerInfo[] = [];
    const board = makeBoard([makeCard("T-7", null)]);

    const result = resolveChatLink("agent-T-7-legacysuffix", planners, board);

    expect(result).toEqual({
      kind: "unknown",
      chatId: "agent-T-7-legacysuffix",
    });
  });

  it("returns unknown when legacy agent-prefix references a non-existent card", () => {
    const planners: PlannerInfo[] = [];
    const board = makeBoard([makeCard("T-1", null)]);

    const result = resolveChatLink("agent-T-99-ghost", planners, board);

    expect(result).toEqual({ kind: "unknown", chatId: "agent-T-99-ghost" });
  });

  it("returns unknown when chatId doesn't match anything", () => {
    const planners = [makePlanner("planner-abc")];
    const board = makeBoard([makeCard("T-1", "agent-T-1-1234abcd")]);

    const result = resolveChatLink("totally-unknown-chat", planners, board);

    expect(result).toEqual({ kind: "unknown", chatId: "totally-unknown-chat" });
  });

  it("returns unknown when board is null", () => {
    const planners = [makePlanner("planner-abc")];

    const result = resolveChatLink("agent-T-1-anything", planners, null);

    expect(result).toEqual({ kind: "unknown", chatId: "agent-T-1-anything" });
  });

  it("returns unknown when board is undefined", () => {
    const planners: PlannerInfo[] = [];

    const result = resolveChatLink("agent-T-1-anything", planners, undefined);

    expect(result).toEqual({ kind: "unknown", chatId: "agent-T-1-anything" });
  });

  it("does not crash on edge-case chatIds with agent- prefix but no suffix", () => {
    const planners: PlannerInfo[] = [];
    const board = makeBoard([makeCard("T-1", null)]);

    // "agent-T-1" has no second dash → lastDashIdx pulled from "T-1" is 1 (> 0),
    // cardId becomes "T", which doesn't match T-1.
    const result = resolveChatLink("agent-T-1", planners, board);

    expect(result.kind).toBe("unknown");
  });

  it("does not crash on empty chatId", () => {
    const result = resolveChatLink("", [], null);
    expect(result).toEqual({ kind: "unknown", chatId: "" });
  });
});

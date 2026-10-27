import type { BoardCard, TaskBoard } from "../../services/refact/tasks";

export interface CardDraft {
  id: string;
  title: string;
  priority: string;
  instructions: string;
  targetFiles: string;
  dependsOn: string[];
}

export type CardEditTarget =
  | { mode: "create" }
  | { mode: "edit" | "move" | "delete"; card: BoardCard };

export function initialCardDraft(target: CardEditTarget): CardDraft {
  const card = target.mode === "create" ? null : target.card;
  return {
    id: card?.id ?? "",
    title: card?.title ?? "",
    priority: card?.priority ?? "P1",
    instructions: card?.instructions ?? "",
    targetFiles: card?.target_files.join("\n") ?? "",
    dependsOn: [...(card?.depends_on ?? [])],
  };
}

export function cardDraftError(
  draft: CardDraft,
  board: TaskBoard,
  creating: boolean,
): string | null {
  const id = creating ? draft.id.trim() : draft.id;
  if (!id || !draft.title.trim()) return "Card ID and title are required.";
  if (creating && board.cards.some((card) => card.id === id))
    return `Card ${id} already exists.`;
  if (!creating && !board.cards.some((card) => card.id === id))
    return `Card ${id} no longer exists. Your draft has been kept.`;
  const byId = new Map(board.cards.map((card) => [card.id, card]));
  for (const dependency of draft.dependsOn) {
    if (dependency === id) return "A card cannot depend on itself.";
    if (!byId.has(dependency)) return `Unknown dependency: ${dependency}.`;
    const pending = [dependency];
    const visited = new Set<string>();
    while (pending.length > 0) {
      const next = pending.pop();
      if (next === undefined || visited.has(next)) continue;
      if (next === id) return "These dependencies would create a cycle.";
      visited.add(next);
      pending.push(...(byId.get(next)?.depends_on ?? []));
    }
  }
  return null;
}

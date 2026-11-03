export type ComposerRow = "top" | "bottom";

export type ComposerLayoutStep =
  | { readonly kind: "shrink"; readonly id: string; readonly level: number }
  | { readonly kind: "hide"; readonly ids: readonly string[] };

export const COMPOSER_LAYOUT_SEQUENCE: readonly ComposerLayoutStep[] = [
  { kind: "shrink", id: "quota", level: 1 },
  { kind: "shrink", id: "skills", level: 1 },
  { kind: "shrink", id: "worktree", level: 1 },
  { kind: "shrink", id: "mode", level: 1 },
  { kind: "shrink", id: "model", level: 1 },
  { kind: "shrink", id: "model", level: 2 },
  { kind: "hide", ids: ["threadInfo"] },
  { kind: "hide", ids: ["sidebar"] },
  { kind: "hide", ids: ["wand"] },
  { kind: "hide", ids: ["autoCompact"] },
  { kind: "hide", ids: ["enrichment"] },
  { kind: "hide", ids: ["browser"] },
  { kind: "hide", ids: ["projectInfo", "selectedLines", "activeFile"] },
  { kind: "hide", ids: ["trajectory"] },
  { kind: "hide", ids: ["quota"] },
  { kind: "hide", ids: ["streaming"] },
  { kind: "hide", ids: ["mic"] },
  { kind: "hide", ids: ["image"] },
  { kind: "hide", ids: ["skills"] },
  { kind: "hide", ids: ["dangerousApproval"] },
  { kind: "hide", ids: ["editApproval"] },
  { kind: "hide", ids: ["worktree"] },
  { kind: "hide", ids: ["context"] },
  { kind: "shrink", id: "mode", level: 2 },
  { kind: "hide", ids: ["mode"] },
];

export type ComposerLayoutItem = {
  id: string;
  row: ComposerRow;
  pinned: boolean;
  widths: readonly number[];
};

export type ComposerLayoutRow = {
  capacity: number;
  gap: number;
};

export type ComposerLayoutPlan = {
  levels: Map<string, number>;
  hidden: Set<string>;
  overflowing: Set<ComposerRow>;
};

const OVERFLOW_TOLERANCE_PX = 0.5;
const ROWS: readonly ComposerRow[] = ["top", "bottom"];

export function maxShrinkLevels(
  sequence: readonly ComposerLayoutStep[] = COMPOSER_LAYOUT_SEQUENCE,
): Map<string, number> {
  const levels = new Map<string, number>();
  for (const step of sequence) {
    if (step.kind !== "shrink") continue;
    levels.set(step.id, Math.max(levels.get(step.id) ?? 0, step.level));
  }
  return levels;
}

export function planComposerLayout(
  items: readonly ComposerLayoutItem[],
  rows: Readonly<Record<ComposerRow, ComposerLayoutRow>>,
  sequence: readonly ComposerLayoutStep[] = COMPOSER_LAYOUT_SEQUENCE,
): ComposerLayoutPlan {
  const levels = new Map<string, number>();
  const hidden = new Set<string>();
  const byId = new Map(items.map((item) => [item.id, item]));

  const widthOf = (item: ComposerLayoutItem) => {
    const level = Math.min(levels.get(item.id) ?? 0, item.widths.length - 1);
    return item.widths[level] ?? 0;
  };

  const required = (row: ComposerRow) => {
    let total = 0;
    let count = 0;
    for (const item of items) {
      if (item.row !== row || hidden.has(item.id)) continue;
      const width = widthOf(item);
      if (width <= 0) continue;
      total += width;
      count += 1;
    }
    return total + Math.max(0, count - 1) * rows[row].gap;
  };

  const overflows = (row: ComposerRow) =>
    required(row) > rows[row].capacity + OVERFLOW_TOLERANCE_PX;

  for (const step of sequence) {
    if (!ROWS.some(overflows)) break;

    if (step.kind === "shrink") {
      const item = byId.get(step.id);
      if (!item || hidden.has(item.id)) continue;
      if (step.level >= item.widths.length) continue;
      if ((levels.get(item.id) ?? 0) >= step.level) continue;
      if (!overflows(item.row)) continue;
      levels.set(item.id, step.level);
      continue;
    }

    const targets = step.ids
      .map((id) => byId.get(id))
      .filter(
        (item): item is ComposerLayoutItem =>
          item !== undefined &&
          !item.pinned &&
          !hidden.has(item.id) &&
          widthOf(item) > 0 &&
          overflows(item.row),
      );
    for (const item of targets) hidden.add(item.id);
  }

  return {
    levels,
    hidden,
    overflowing: new Set(ROWS.filter(overflows)),
  };
}

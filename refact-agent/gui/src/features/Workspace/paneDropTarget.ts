import type { Point } from "../ChatPanes/pointerDrag";

export type PaneDropTarget = "left" | "right" | "top" | "bottom" | "center";
type PaneBounds = { left: number; top: number; width: number; height: number };

export function resolvePaneDropTarget(
  bounds: PaneBounds,
  point: Point,
): PaneDropTarget | null {
  if (bounds.width <= 0 || bounds.height <= 0) return null;
  const x = (point.x - bounds.left) / bounds.width;
  const y = (point.y - bounds.top) / bounds.height;
  if (![x, y].every(Number.isFinite) || x < 0 || x > 1 || y < 0 || y > 1) {
    return null;
  }
  const edges = [
    ["left", x],
    ["right", 1 - x],
    ["top", y],
    ["bottom", 1 - y],
  ] as const;
  let closest: (typeof edges)[number] = edges[0];
  for (const edge of edges) {
    if (edge[1] < closest[1]) closest = edge;
  }
  return closest[1] <= 0.25 ? closest[0] : "center";
}

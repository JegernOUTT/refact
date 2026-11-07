export type PaneTabFitChange = (
  element: HTMLElement,
  naturalWidth: number | null,
) => void;

export function fitPaneTabs(
  minimums: number[],
  available: number,
  overflowWidth: number,
  activeIndex: number,
): number[] {
  if (minimums.reduce((sum, width) => sum + width, 0) <= available)
    return minimums.map((_, index) => index);
  if (minimums.length === 0) return [];
  const active = Math.max(0, activeIndex);
  const selected = new Set([active]);
  let remaining = available - overflowWidth - minimums[active];
  minimums.forEach((width, index) => {
    if (index !== active && width <= remaining) {
      selected.add(index);
      remaining -= width;
    }
  });
  return minimums.flatMap((_, index) => (selected.has(index) ? [index] : []));
}

export function needsCompactEngine(
  strips: { left: number; right: number; naturalWidth: number }[],
  expandedSlotRight: number,
): boolean {
  return strips.some(
    ({ left, right, naturalWidth }) =>
      naturalWidth > Math.max(0, Math.min(right, expandedSlotRight) - left),
  );
}

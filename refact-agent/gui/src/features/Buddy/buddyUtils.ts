export function computeXpFill(xp: number, xpNext: number): number {
  if (xpNext <= 0) return 100;
  return Math.min(100, Math.max(0, (xp / xpNext) * 100));
}

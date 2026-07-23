export function formatCostTick(value: number, maxValue: number): string {
  if (maxValue < 1) {
    const precise = value.toFixed(3);
    return `$${precise.endsWith("0") ? value.toFixed(2) : precise}`;
  }
  if (maxValue < 10) return `$${value.toFixed(2)}`;
  return `$${value.toFixed(0)}`;
}

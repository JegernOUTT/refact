export function summarizeServerBlocks(blocks: unknown[]): string {
  const counts = new Map<string, number>();
  for (const block of blocks) {
    const type =
      block !== null &&
      typeof block === "object" &&
      "type" in block &&
      typeof block.type === "string" &&
      block.type
        ? block.type
        : "unknown";
    counts.set(type, (counts.get(type) ?? 0) + 1);
  }
  const parts = [...counts.entries()].map(([type, count]) =>
    count > 1 ? `${type} ×${count}` : type,
  );
  const noun = blocks.length === 1 ? "block" : "blocks";
  return `${blocks.length} provider ${noun}: ${parts.join(", ")}`;
}

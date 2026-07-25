import type { DiffChunk } from "../../../services/refact";

const normalizeNewlines = (value: string): string =>
  value.replace(/\r\n/g, "\n");

const lineCount = (value: string): number => {
  if (!value) return 0;
  const normalized = normalizeNewlines(value);
  return normalized.endsWith("\n")
    ? normalized.slice(0, -1).split("\n").length
    : normalized.split("\n").length;
};

export const changedLineNumbers = (chunks: DiffChunk[]): number[] => {
  const lines = new Set<number>();
  for (const chunk of chunks) {
    const count = Math.max(1, lineCount(chunk.lines_add));
    for (let offset = 0; offset < count; offset += 1) {
      lines.add(chunk.line1 + offset);
    }
  }
  return [...lines];
};

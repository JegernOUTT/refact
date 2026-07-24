import type { DiffChunk } from "../../../services/refact";
import type { LiveFileUpdate } from "./filesPanelSlice";

const normalizeNewlines = (value: string): string =>
  value.replace(/\r\n/g, "\n");

const insertionOffset = (content: string, lineNumber: number): number => {
  if (lineNumber <= 1) return 0;
  let offset = 0;
  for (let line = 1; line < lineNumber; line += 1) {
    const newline = content.indexOf("\n", offset);
    if (newline === -1) return content.length;
    offset = newline + 1;
  }
  return offset;
};

const applyChunk = (content: string, chunk: DiffChunk): string => {
  const removed = normalizeNewlines(chunk.lines_remove);
  const added = normalizeNewlines(chunk.lines_add);
  if (!removed) {
    if (!added || content.includes(added)) return content;
    const offset = insertionOffset(content, chunk.line1);
    return `${content.slice(0, offset)}${added}${content.slice(offset)}`;
  }

  const expectedOffset = insertionOffset(content, chunk.line1);
  const atExpectedOffset = content.slice(
    expectedOffset,
    expectedOffset + removed.length,
  );
  if (atExpectedOffset === removed) {
    return `${content.slice(0, expectedOffset)}${added}${content.slice(
      expectedOffset + removed.length,
    )}`;
  }

  const removedOffset = content.indexOf(removed);
  if (removedOffset >= 0) {
    return `${content.slice(0, removedOffset)}${added}${content.slice(
      removedOffset + removed.length,
    )}`;
  }

  return added && content.includes(added) ? content : content;
};

export const applyLiveFileUpdates = (
  baseContent: string,
  updates: LiveFileUpdate[],
): string =>
  updates.reduce(
    (content, update) =>
      update.fileAfter ??
      update.chunks.reduce(
        (updatedContent, chunk) => applyChunk(updatedContent, chunk),
        content,
      ),
    normalizeNewlines(baseContent),
  );

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

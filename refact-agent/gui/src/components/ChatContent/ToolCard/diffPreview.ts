import type { DiffChunk } from "../../../services/refact/types";

export const MAX_DIFF_INPUT_CHARS = 100_000;
export const MAX_DIFF_INPUT_LINES = 2_000;
export const MAX_DIFF_PREVIEW_CHARS = 20_000;
export const MAX_DIFF_PREVIEW_LINES = 240;

const TEXT_FIELDS = [
  "lines_before",
  "lines_remove",
  "lines_add",
  "lines_after",
] as const;

export function isOversizedDiff(diff: DiffChunk): boolean {
  const chars = TEXT_FIELDS.reduce(
    (total, field) => total + (diff[field]?.length ?? 0),
    0,
  );
  if (chars > MAX_DIFF_INPUT_CHARS) return true;

  let lines = 0;
  for (const field of TEXT_FIELDS) {
    const text = diff[field] ?? "";
    if (text) lines++;
    for (let i = 0; i < text.length; i++) {
      if (text[i] === "\r" || text[i] === "\n") {
        if (text[i] === "\r" && text[i + 1] === "\n") i++;
        if (i + 1 < text.length) lines++;
      }
      if (lines > MAX_DIFF_INPUT_LINES) return true;
    }
  }
  return false;
}

export function limitDiffPreview(diff: DiffChunk): DiffChunk {
  const preview = { ...diff };
  const fieldCount = TEXT_FIELDS.filter((field) => diff[field]?.length).length;
  const charsPerField = Math.floor(
    MAX_DIFF_PREVIEW_CHARS / Math.max(1, fieldCount),
  );
  const linesPerField = Math.floor(
    MAX_DIFF_PREVIEW_LINES / Math.max(1, fieldCount),
  );
  for (const field of TEXT_FIELDS) {
    const text = diff[field] ?? "";
    const max = Math.min(text.length, charsPerField);
    let linesLeft = linesPerField;
    let end = 0;
    while (end < max && linesLeft > 0) {
      const char = text[end++];
      if (char === "\r" || char === "\n") {
        if (char === "\r" && text[end] === "\n" && end < max) end++;
        linesLeft--;
      }
    }
    preview[field] = text.slice(0, end);
  }
  return preview;
}

export function downloadOriginalDiff(diff: DiffChunk): void {
  const blob = new Blob([JSON.stringify(diff)], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  try {
    link.href = url;
    link.download = "original-diff-chunk.json";
    document.body.appendChild(link);
    link.click();
  } finally {
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1_000);
  }
}

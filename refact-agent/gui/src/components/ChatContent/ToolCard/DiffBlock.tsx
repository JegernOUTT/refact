import React, { useEffect, useMemo, useState } from "react";
import { Flex, Text, Box } from "@radix-ui/themes";
import { useAppearance, useShiki } from "../../../hooks";
import { DiffChunk } from "../../../services/refact/types";
import { basename } from "./utils";
import { extractCodeLines } from "./editToolHighlight";
import styles from "./EditTool.module.css";

function countNonEmptyLines(text: string): number {
  let count = 0;
  let hasContent = false;

  for (const char of text) {
    if (char === "\n") {
      if (hasContent) count++;
      hasContent = false;
    } else if (char !== "\r" && char !== " " && char !== "\t") {
      hasContent = true;
    }
  }

  return hasContent ? count + 1 : count;
}

function getDiffStats(diffs: DiffChunk[]): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const diff of diffs) {
    added += countNonEmptyLines(diff.lines_add);
    removed += countNonEmptyLines(diff.lines_remove);
  }
  return { added, removed };
}

const EXTENSION_LANGUAGE_MAP: Partial<Record<string, string>> = {
  js: "javascript",
  jsx: "jsx",
  ts: "typescript",
  tsx: "tsx",
  py: "python",
  rs: "rust",
  go: "go",
  java: "java",
  c: "c",
  h: "c",
  cpp: "cpp",
  cc: "cpp",
  cxx: "cpp",
  hpp: "cpp",
  cs: "csharp",
  html: "html",
  css: "css",
  json: "json",
  yaml: "yaml",
  yml: "yaml",
  md: "markdown",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  sql: "sql",
  dockerfile: "dockerfile",
};

function languageForFile(fileName: string | undefined): string {
  if (!fileName) return "text";
  const lowerFileName = fileName.toLowerCase();
  if (lowerFileName.endsWith("dockerfile")) return "dockerfile";
  const parts = lowerFileName.split(".");
  const extension = parts[parts.length - 1];
  if (!extension) return "text";
  return EXTENSION_LANGUAGE_MAP[extension] ?? "text";
}

const CONTEXT_LINES = 1;
const MAX_VISIBLE_DIFF_LINES = 240;
const MAX_HIGHLIGHT_CHARS = 50_000;

type DiffLineKind = "context" | "remove" | "add";

type RenderedDiffLine = {
  kind: DiffLineKind;
  oldLineNumber?: number;
  newLineNumber?: number;
  sign: string;
  line: string;
};

type RenderedDiffHunk = {
  header: string;
  lines: RenderedDiffLine[];
};

export type DiffHeaderAction = {
  label: string;
  icon: React.ReactNode;
  onClick: () => void;
  disabled?: boolean;
};

type HighlightedLine = {
  html: string;
  text: string;
};

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function useHighlightedLines(
  lines: RenderedDiffLine[],
  language: string,
): HighlightedLine[] {
  const { highlight, isReady } = useShiki();
  const { appearance } = useAppearance();
  const [highlightedLines, setHighlightedLines] = useState<
    HighlightedLine[] | null
  >(null);
  const shouldHighlight =
    lines.reduce((total, line) => total + line.line.length, 0) <=
    MAX_HIGHLIGHT_CHARS;
  const highlightKey = useMemo(
    () => lines.map((line) => `${line.kind}:${line.line}`).join("\n\u0000\n"),
    [lines],
  );
  useEffect(() => {
    if (!isReady || !shouldHighlight || lines.length === 0) {
      setHighlightedLines(null);
      return;
    }

    let cancelled = false;
    const timer = setTimeout(() => {
      Promise.all(
        lines.map((line) =>
          highlight(line.line, language, appearance === "dark"),
        ),
      )
        .then((results) => {
          if (cancelled) return;
          const next = lines.map((line) => ({
            text: line.line,
            html: escapeHtml(line.line),
          }));
          results.forEach((result, index) => {
            const line = lines[index];
            const htmlLines = extractCodeLines(result.html);
            next[index] = {
              text: line.line,
              html: htmlLines[0] ?? escapeHtml(line.line),
            };
          });
          setHighlightedLines(next);
        })
        .catch(() => {
          if (!cancelled) setHighlightedLines(null);
        });
    }, 300);

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [
    appearance,
    highlight,
    highlightKey,
    isReady,
    language,
    lines,
    shouldHighlight,
  ]);

  return (
    highlightedLines ??
    lines.map((line) => ({
      text: line.line,
      html: escapeHtml(line.line),
    }))
  );
}

function displayLineNumber(line: RenderedDiffLine): number | undefined {
  if (line.kind === "add") return line.newLineNumber;
  if (line.kind === "context") {
    if (line.oldLineNumber === undefined) return line.newLineNumber;
    if (line.newLineNumber === undefined) return line.oldLineNumber;
    return contextLineNumber(line.oldLineNumber, line.newLineNumber);
  }
  return line.oldLineNumber;
}

const DiffLine: React.FC<
  RenderedDiffLine & { highlighted: HighlightedLine }
> = (line) => {
  const { kind, sign, highlighted } = line;
  const rowClass =
    kind === "remove"
      ? styles.remove
      : kind === "add"
        ? styles.add
        : styles.context;
  return (
    <div className={`${styles.diffLine} ${rowClass}`}>
      <span className={styles.lineNumber}>{displayLineNumber(line) ?? ""}</span>
      <span className={styles.sign}>{sign}</span>
      <code
        className={styles.lineContent}
        dangerouslySetInnerHTML={{ __html: highlighted.html }}
      />
    </div>
  );
};

function splitDiffLines(text: string): string[] {
  if (!text) return [];

  const normalized = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const lines = normalized.split("\n");
  if (normalized.endsWith("\n")) lines.pop();
  return lines;
}

function firstContextLine(text: string | null | undefined): string | null {
  const lines = splitDiffLines(text ?? "");
  return lines[0] ?? null;
}

function commonPrefixLength(left: string[], right: string[]): number {
  const max = Math.min(left.length, right.length);
  let count = 0;
  while (count < max && left[count] === right[count]) count++;
  return count;
}

function commonSuffixLength(
  left: string[],
  right: string[],
  prefixLength: number,
): number {
  const max = Math.min(left.length, right.length) - prefixLength;
  let count = 0;
  while (
    count < max &&
    left[left.length - 1 - count] === right[right.length - 1 - count]
  ) {
    count++;
  }
  return count;
}

function lineSpan(start: number, count: number): string {
  return count === 1 ? String(start) : `${start},${count}`;
}

function formatHunkHeader(
  diff: DiffChunk,
  oldLineCount: number,
  newLineCount: number,
): string {
  return `@@ -${lineSpan(diff.line1, oldLineCount)} +${lineSpan(
    diff.line2,
    newLineCount,
  )} @@`;
}

function contextLineNumber(
  oldLineNumber: number,
  newLineNumber: number,
): number {
  return oldLineNumber === newLineNumber ? oldLineNumber : newLineNumber;
}

function buildRenderedHunk(diff: DiffChunk): RenderedDiffHunk {
  const removeLines = splitDiffLines(diff.lines_remove);
  const addLines = splitDiffLines(diff.lines_add);
  const prefixLength = commonPrefixLength(removeLines, addLines);
  const suffixLength = commonSuffixLength(removeLines, addLines, prefixLength);
  const lines: RenderedDiffLine[] = [];
  const backendBeforeLine = firstContextLine(diff.lines_before);
  if (backendBeforeLine !== null) {
    lines.push({
      kind: "context",
      oldLineNumber: diff.line1 > 1 ? diff.line1 - 1 : undefined,
      newLineNumber: diff.line2 > 1 ? diff.line2 - 1 : undefined,
      sign: " ",
      line: backendBeforeLine,
    });
  }

  const inferredBeforeContextLines =
    backendBeforeLine === null ? CONTEXT_LINES : 0;
  const beforeContextStart = Math.max(
    0,
    prefixLength - inferredBeforeContextLines,
  );
  for (let i = beforeContextStart; i < prefixLength; i++) {
    lines.push({
      kind: "context",
      oldLineNumber: diff.line1 + i,
      newLineNumber: diff.line2 + i,
      sign: " ",
      line: removeLines[i],
    });
  }

  const removeChangeEnd = removeLines.length - suffixLength;
  for (let i = prefixLength; i < removeChangeEnd; i++) {
    lines.push({
      kind: "remove",
      oldLineNumber: diff.line1 + i,
      sign: "-",
      line: removeLines[i],
    });
  }

  const addChangeEnd = addLines.length - suffixLength;
  for (let i = prefixLength; i < addChangeEnd; i++) {
    lines.push({
      kind: "add",
      newLineNumber: diff.line2 + i,
      sign: "+",
      line: addLines[i],
    });
  }

  const backendAfterLine = firstContextLine(diff.lines_after);
  const suffixStart = removeLines.length - suffixLength;
  const inferredAfterContextLines =
    backendAfterLine === null ? CONTEXT_LINES : 0;
  const afterContextEnd = Math.min(
    removeLines.length,
    suffixStart + inferredAfterContextLines,
  );
  for (let i = suffixStart; i < afterContextEnd; i++) {
    lines.push({
      kind: "context",
      oldLineNumber: diff.line1 + i,
      newLineNumber: diff.line2 + i,
      sign: " ",
      line: removeLines[i],
    });
  }

  if (backendAfterLine !== null) {
    lines.push({
      kind: "context",
      oldLineNumber: diff.line1 + removeLines.length,
      newLineNumber: diff.line2 + addLines.length,
      sign: " ",
      line: backendAfterLine,
    });
  }

  return {
    header: formatHunkHeader(diff, removeLines.length, addLines.length),
    lines,
  };
}

export const DiffBlock: React.FC<{
  diff: DiffChunk;
  fileName?: string;
  displayFileName?: string;
  onOpenFile?: () => void;
  actions?: DiffHeaderAction[];
}> = ({ diff, fileName, displayFileName, onOpenFile, actions = [] }) => {
  const [showAll, setShowAll] = useState(false);
  const hunk = useMemo(() => buildRenderedHunk(diff), [diff]);
  const stats = useMemo(() => getDiffStats([diff]), [diff]);
  const isLarge = hunk.lines.length > MAX_VISIBLE_DIFF_LINES;
  const visibleLines = useMemo(
    () => (showAll ? hunk.lines : hunk.lines.slice(0, MAX_VISIBLE_DIFF_LINES)),
    [hunk.lines, showAll],
  );
  const highlightedLines = useHighlightedLines(
    visibleLines,
    languageForFile(fileName ?? diff.file_name),
  );
  const hiddenLineCount = Math.max(0, hunk.lines.length - visibleLines.length);

  return (
    <Box className={styles.diffBlock}>
      <Flex className={styles.hunkHeader} align="center" gap="2">
        {fileName && onOpenFile && (
          <button
            type="button"
            className={styles.hunkFileButton}
            onClick={onOpenFile}
            title={fileName}
          >
            {displayFileName ?? basename(fileName)}
          </button>
        )}
        {fileName && !onOpenFile && (
          <span className={styles.hunkFileName} title={fileName}>
            {displayFileName ?? basename(fileName)}
          </span>
        )}
        <Text size="1" className={styles.hunkTitle}>
          {hunk.header}
        </Text>
        <Text size="1" className={styles.statsInline}>
          {stats.added > 0 && (
            <span className={styles.added}>+{stats.added}</span>
          )}
          {stats.removed > 0 && (
            <span className={styles.removed}>−{stats.removed}</span>
          )}
        </Text>
        {actions.length > 0 && (
          <Flex gap="1" className={styles.hunkActions}>
            {actions.map((action) => (
              <button
                key={action.label}
                type="button"
                className={`${styles.hunkActionButton} rf-pressable`}
                onClick={action.onClick}
                disabled={action.disabled}
                title={action.label}
                aria-label={action.label}
              >
                {action.icon}
              </button>
            ))}
          </Flex>
        )}
      </Flex>
      {visibleLines.map((line, i) => (
        <DiffLine
          key={`${line.kind}-${line.oldLineNumber ?? ""}-${
            line.newLineNumber ?? ""
          }-${i}`}
          {...line}
          highlighted={
            highlightedLines[i] ?? {
              text: line.line,
              html: escapeHtml(line.line),
            }
          }
        />
      ))}
      {isLarge && (
        <button
          type="button"
          className={styles.showMoreButton}
          onClick={() => setShowAll((prev) => !prev)}
        >
          {showAll
            ? "Show fewer diff lines"
            : `Show ${hiddenLineCount} more diff lines`}
        </button>
      )}
    </Box>
  );
};

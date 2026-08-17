import { useEffect, useMemo, useState } from "react";

import { extractCodeLines } from "../../../components/ChatContent/ToolCard/editToolHighlight";
import { useAppearance, useShiki } from "../../../hooks";
import type { DiffChunk } from "../../../services/refact";
import styles from "./FilesPanel.module.css";

const MAX_HIGHLIGHT_CHARS = 50_000;
const MAX_GHOST_LINES_PER_CHUNK = 40;

const escapeHtml = (value: string): string =>
  value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");

type RemovedRow = {
  kind: "removed";
  key: string;
  text: string;
};

type SourceRow = {
  kind: "source";
  key: string;
  index: number;
  lineNumber: number;
};

type Row = RemovedRow | SourceRow;

const removedLinesByAnchor = (chunks: DiffChunk[]): Map<number, string[]> => {
  const byAnchor = new Map<number, string[]>();
  for (const chunk of chunks) {
    const removed = chunk.lines_remove.replace(/\n$/, "");
    if (!removed) continue;
    const lines = removed.split("\n").slice(0, MAX_GHOST_LINES_PER_CHUNK);
    byAnchor.set(chunk.line1, [...(byAnchor.get(chunk.line1) ?? []), ...lines]);
  }
  return byAnchor;
};

export function HighlightedFile({
  content,
  changedLines = [],
  changeRevision,
  language,
  lineStart,
  removedChunks = [],
  targetLine,
}: {
  content: string;
  changedLines?: number[];
  changeRevision?: string;
  language: string | null;
  lineStart: number;
  removedChunks?: DiffChunk[];
  targetLine?: number;
}) {
  const { highlight } = useShiki();
  const { appearance } = useAppearance();
  const [highlightedLines, setHighlightedLines] = useState<string[] | null>(
    null,
  );
  const [visibleChangedLines, setVisibleChangedLines] = useState<number[]>([]);
  const sourceLines = useMemo(
    () => content.replace(/\n$/, "").split("\n"),
    [content],
  );

  useEffect(() => {
    let cancelled = false;
    setHighlightedLines(null);
    if (content.length > MAX_HIGHLIGHT_CHARS) return undefined;
    void highlight(content, language ?? "plaintext", appearance === "dark")
      .then((result) => {
        if (!cancelled) setHighlightedLines(extractCodeLines(result.html));
      })
      .catch(() => {
        if (!cancelled) setHighlightedLines(null);
      });
    return () => {
      cancelled = true;
    };
  }, [appearance, content, highlight, language]);

  useEffect(() => {
    if (!changeRevision || changedLines.length === 0) {
      setVisibleChangedLines([]);
      return;
    }
    setVisibleChangedLines(changedLines);
    const timer = window.setTimeout(() => setVisibleChangedLines([]), 1800);
    return () => window.clearTimeout(timer);
  }, [changeRevision, changedLines]);

  const visibleChanges = useMemo(
    () => new Set(visibleChangedLines),
    [visibleChangedLines],
  );

  const ghostLines = useMemo(
    () =>
      changeRevision && visibleChangedLines.length > 0
        ? removedLinesByAnchor(removedChunks)
        : new Map<number, string[]>(),
    [changeRevision, removedChunks, visibleChangedLines.length],
  );

  const rows = useMemo(() => {
    const built: Row[] = [];
    sourceLines.forEach((_line, index) => {
      const lineNumber = lineStart + index;
      for (const [offset, text] of (
        ghostLines.get(lineNumber) ?? []
      ).entries()) {
        built.push({
          kind: "removed",
          key: `removed-${lineNumber}-${offset}`,
          text,
        });
      }
      built.push({
        kind: "source",
        key: `source-${lineNumber}`,
        index,
        lineNumber,
      });
    });
    return built;
  }, [ghostLines, lineStart, sourceLines]);

  return (
    <div className={styles.codeTable} role="table">
      {rows.map((row) => {
        if (row.kind === "removed") {
          return (
            <div
              aria-hidden="true"
              className={styles.removedLine}
              data-live-removed="true"
              key={`${row.key}-${changeRevision ?? ""}`}
              role="row"
            >
              <span className={styles.lineNumber} role="cell" />
              <code className={styles.lineCode} role="cell">
                {row.text || " "}
              </code>
            </div>
          );
        }
        const target = row.lineNumber === targetLine;
        return (
          <div
            className={styles.codeLine}
            data-line-number={row.lineNumber}
            data-live-change={
              visibleChanges.has(row.lineNumber) ? "true" : undefined
            }
            data-target-line={target ? "true" : undefined}
            id={target ? "files-panel-target-line" : undefined}
            key={row.key}
            role="row"
          >
            <span className={styles.lineNumber} role="cell">
              {row.lineNumber}
            </span>
            <code
              className={styles.lineCode}
              dangerouslySetInnerHTML={{
                __html:
                  highlightedLines?.[row.index] ??
                  (escapeHtml(sourceLines[row.index]) || " "),
              }}
              role="cell"
            />
          </div>
        );
      })}
    </div>
  );
}

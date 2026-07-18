import { useEffect, useMemo, useState } from "react";

import { extractCodeLines } from "../../../components/ChatContent/ToolCard/editToolHighlight";
import { useAppearance, useShiki } from "../../../hooks";
import styles from "./FilesPanel.module.css";

const MAX_HIGHLIGHT_CHARS = 50_000;

const escapeHtml = (value: string): string =>
  value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");

export function HighlightedFile({
  content,
  language,
  lineStart,
  targetLine,
}: {
  content: string;
  language: string | null;
  lineStart: number;
  targetLine?: number;
}) {
  const { highlight } = useShiki();
  const { appearance } = useAppearance();
  const [highlightedLines, setHighlightedLines] = useState<string[] | null>(
    null,
  );
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

  return (
    <div className={styles.codeTable} role="table">
      {sourceLines.map((line, index) => {
        const lineNumber = lineStart + index;
        const target = lineNumber === targetLine;
        return (
          <div
            className={styles.codeLine}
            data-line-number={lineNumber}
            data-target-line={target ? "true" : undefined}
            id={target ? "files-panel-target-line" : undefined}
            key={lineNumber}
            role="row"
          >
            <span className={styles.lineNumber} role="cell">
              {lineNumber}
            </span>
            <code
              className={styles.lineCode}
              dangerouslySetInnerHTML={{
                __html: highlightedLines?.[index] ?? (escapeHtml(line) || " "),
              }}
              role="cell"
            />
          </div>
        );
      })}
    </div>
  );
}

import React, { CSSProperties, useEffect, useState, useMemo } from "react";
import classNames from "classnames";
import { PreTag, PreTagProps } from "./Pre";
import styles from "./Markdown.module.css";
import type { Element } from "hast";
import { trimIndent } from "../../utils";
import { useShiki } from "../../hooks/useShiki";
import { useAppearance } from "../../hooks/useAppearance";
import { MermaidBlock } from "./MermaidBlock";
import { SvgBlock } from "./SvgBlock";
import { ArtifactBlock } from "./ArtifactBlock";

const DIAGRAM_LANGUAGES = new Set(["mermaid", "svg"]);
const ARTIFACT_LANGUAGES = new Set(["html"]);

export type MarkdownControls = {
  onCopyClick: (str: string) => void;
};

export type ShikiCodeBlockProps = React.JSX.IntrinsicElements["code"] & {
  node?: Element | undefined;
  style?: CSSProperties;
  wrap?: boolean;
  preOptions?: {
    noMargin?: boolean;
    widthMaxContent?: boolean;
  };
  color?: string;
  showLineNumbers?: boolean;
  isStreaming?: boolean;
} & Partial<MarkdownControls>;

const MAX_HIGHLIGHT_CHARS = 50000;

const _ShikiCodeBlock: React.FC<ShikiCodeBlockProps> = ({
  children,
  className,
  onCopyClick,
  wrap = false,
  preOptions = { widthMaxContent: false, noMargin: false },
  color = undefined,
  showLineNumbers = false,
  isStreaming,
}) => {
  const codeRef = React.useRef<HTMLElement | null>(null);
  const { highlight, isReady } = useShiki();
  const { appearance } = useAppearance();
  const [highlightedHtml, setHighlightedHtml] = useState<string | null>(null);

  const match = /language-([^\s]+)/.exec(className ?? "");
  const textWithOutTrailingNewLine =
    children === undefined ? undefined : String(children).replace(/\n$/, "");
  const textWithOutIndent = trimIndent(textWithOutTrailingNewLine);

  const isBlock = match !== null || String(children).includes("\n");
  const language: string = match?.[1] ?? "text";
  const isDark = appearance === "dark";

  const isSpecialBlock =
    isBlock &&
    (DIAGRAM_LANGUAGES.has(language) || ARTIFACT_LANGUAGES.has(language));

  const shouldHighlight =
    isBlock &&
    !isSpecialBlock &&
    !isStreaming &&
    isReady &&
    textWithOutIndent &&
    textWithOutIndent.length <= MAX_HIGHLIGHT_CHARS;

  useEffect(() => {
    if (!shouldHighlight || !textWithOutIndent) {
      setHighlightedHtml(null);
      return;
    }

    let cancelled = false;
    const timer = setTimeout(() => {
      highlight(textWithOutIndent, language, isDark)
        .then((result) => {
          if (!cancelled) {
            setHighlightedHtml(result.html);
          }
        })
        .catch(() => {
          if (!cancelled) {
            setHighlightedHtml(null);
          }
        });
    }, 300);

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [shouldHighlight, textWithOutIndent, language, isDark, highlight]);

  const preTagProps: PreTagProps = useMemo(() => {
    if (onCopyClick && textWithOutIndent) {
      return {
        onCopyClick: () => {
          if (codeRef.current?.textContent) {
            onCopyClick(codeRef.current.textContent);
          }
        },
      };
    }
    return {};
  }, [onCopyClick, textWithOutIndent]);

  if (isBlock && DIAGRAM_LANGUAGES.has(language) && !isStreaming) {
    const diagramCode = textWithOutIndent ?? String(children);
    if (language === "mermaid") {
      return <MermaidBlock code={diagramCode} onCopyClick={onCopyClick} />;
    }
    if (language === "svg") {
      return <SvgBlock code={diagramCode} onCopyClick={onCopyClick} />;
    }
  }

  if (isBlock && ARTIFACT_LANGUAGES.has(language)) {
    const artifactCode = textWithOutIndent ?? String(children);
    return (
      <ArtifactBlock
        code={artifactCode}
        isStreaming={isStreaming}
        onCopyClick={onCopyClick}
      />
    );
  }

  if (!isBlock) {
    return (
      <code
        className={classNames(styles.code, styles.code_inline, className)}
        data-color={color}
      >
        {children}
      </code>
    );
  }

  return (
    <div className={classNames("scrollX", "rf-enter", styles.shiki_wrapper)}>
      <PreTag
        className={classNames({
          [styles.pre_width_max_content]: preOptions.widthMaxContent,
          [styles.code_no_margin]: preOptions.noMargin,
          [styles.shiki_pre]: true,
        })}
        {...preTagProps}
      >
        <div
          className={classNames(styles.shiki_code, wrap && styles.code_wrap)}
        >
          {showLineNumbers && highlightedHtml && (
            <div className={styles.line_numbers}>
              {textWithOutIndent?.split("\n").map((_, i) => (
                <span key={i} className={styles.line_number}>
                  {i + 1}
                </span>
              ))}
            </div>
          )}
          {highlightedHtml ? (
            <code
              ref={codeRef}
              className={classNames(styles.code, styles.code_block)}
              dangerouslySetInnerHTML={{
                __html: stripShikiBackground(
                  extractCodeContent(highlightedHtml),
                ),
              }}
            />
          ) : (
            <code
              className={classNames(
                styles.code,
                styles.code_block,
                wrap && styles.code_wrap,
              )}
              ref={codeRef}
              style={!wrap ? { whiteSpace: "pre" } : undefined}
            >
              {textWithOutIndent}
            </code>
          )}
        </div>
      </PreTag>
    </div>
  );
};

function extractCodeContent(html: string): string {
  const codeMatch = /<code[^>]*>([\s\S]*?)<\/code>/i.exec(html);
  if (codeMatch) {
    return codeMatch[1];
  }
  return html.replace(/<\/?pre[^>]*>/gi, "").replace(/<\/?code[^>]*>/gi, "");
}

function stripShikiBackground(html: string): string {
  return html
    .replace(/style="[^"]*background-color:[^;"]*;?/gi, 'style="')
    .replace(/style="[^"]*background:[^;"]*;?/gi, 'style="')
    .replace(/style="\s*"/g, "");
}

export const ShikiCodeBlock = React.memo(_ShikiCodeBlock);

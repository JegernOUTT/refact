import React, { CSSProperties, useEffect, useState, useMemo } from "react";
import { Code, CodeProps, Box } from "@radix-ui/themes";
import classNames from "classnames";
import { PreTag, PreTagProps } from "./Pre";
import styles from "./Markdown.module.css";
import type { Element } from "hast";
import { trimIndent } from "../../utils";
import { useShiki } from "../../hooks/useShiki";
import { useAppearance } from "../../hooks/useAppearance";

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
  color?: CodeProps["color"];
  showLineNumbers?: boolean;
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

  const shouldHighlight =
    isBlock &&
    isReady &&
    textWithOutIndent &&
    textWithOutIndent.length <= MAX_HIGHLIGHT_CHARS;

  useEffect(() => {
    if (!shouldHighlight || !textWithOutIndent) {
      setHighlightedHtml(null);
      return;
    }

    let cancelled = false;
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

    return () => {
      cancelled = true;
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

  if (!isBlock) {
    return (
      <Code
        variant="ghost"
        className={classNames(styles.code, styles.code_inline, className)}
        color={color}
      >
        {children}
      </Code>
    );
  }

  return (
    <Box className={styles.shiki_wrapper}>
      <PreTag
        className={classNames({
          [styles.pre_width_max_content]: preOptions.widthMaxContent,
          [styles.code_no_margin]: preOptions.noMargin,
          [styles.shiki_pre]: true,
        })}
        {...preTagProps}
      >
        {highlightedHtml ? (
          <div
            className={classNames(styles.shiki_code, wrap && styles.code_wrap)}
          >
            {showLineNumbers && (
              <div className={styles.line_numbers}>
                {textWithOutIndent?.split("\n").map((_, i) => (
                  <span key={i} className={styles.line_number}>
                    {i + 1}
                  </span>
                ))}
              </div>
            )}
            <code
              ref={codeRef}
              className={classNames(styles.code, styles.code_block)}
              dangerouslySetInnerHTML={{
                __html: stripShikiBackground(
                  extractCodeContent(highlightedHtml),
                ),
              }}
            />
          </div>
        ) : (
          <code
            className={classNames(
              styles.code,
              styles.code_block,
              wrap && styles.code_wrap,
            )}
            ref={codeRef}
          >
            {textWithOutIndent}
          </code>
        )}
      </PreTag>
    </Box>
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

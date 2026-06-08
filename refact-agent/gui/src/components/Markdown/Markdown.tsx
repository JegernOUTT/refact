import React, { Key, useMemo } from "react";
import ReactMarkdown, { Components } from "react-markdown";
import remarkBreaks from "remark-breaks";
import classNames from "classnames";
import styles from "./Markdown.module.css";
import {
  ShikiCodeBlock,
  type ShikiCodeBlockProps,
  type MarkdownControls,
} from "./ShikiCodeBlock";
import { Link } from "../Link";
import rehypeKatex from "rehype-katex";
import remarkMath from "remark-math";
import remarkGfm from "remark-gfm";
import "katex/dist/katex.min.css";
import type { PluggableList } from "unified";
import { useLinksFromLsp } from "../../hooks";

const REMARK_PLUGINS: PluggableList = [remarkBreaks, remarkMath, remarkGfm];
const REHYPE_PLUGINS: PluggableList = [rehypeKatex];
const SAFE_URL_PREFIXES = ["refact://", "http://", "https://", "mailto:"];

function transformMarkdownUrl(url: string): string {
  const lowerUrl = url.toLowerCase();
  return SAFE_URL_PREFIXES.some((prefix) => lowerUrl.startsWith(prefix))
    ? url
    : "";
}

import { ChatLinkButton } from "../ChatLinks";
import { extractLinkFromPuzzle } from "../../utils/extractLinkFromPuzzle";
import { useInternalLinkHandler } from "../../contexts/internalLinkUtils";

export type MarkdownProps = Pick<
  React.ComponentProps<typeof ReactMarkdown>,
  "children" | "allowedElements" | "unwrapDisallowed"
> &
  Pick<ShikiCodeBlockProps, "showLineNumbers" | "color" | "isStreaming"> & {
    canHaveInteractiveElements?: boolean;
    wrap?: boolean;
    variant?: "chat" | "tool" | "terminal";
  } & Partial<MarkdownControls>;

const STREAMING_SAFE_FENCE_LANGUAGE = "text";
const STREAMING_SPECIAL_FENCE_LANGUAGES = new Set(["mermaid", "html", "svg"]);

function maskIncompleteSpecialCodeFences(text: string): string {
  const lines = text.split(/(?<=\n)/);
  let inFence = false;
  let fenceChar = "`";
  let fenceLength = 0;
  let specialFenceLineIndex = -1;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].replace(/\r?\n$/, "");

    if (!inFence) {
      const opening = /^( {0,3})(`{3,}|~{3,})([^`~]*)$/.exec(line);
      if (!opening) continue;

      const info = opening[3].trim();
      const language = info.split(/\s+/)[0]?.toLowerCase() ?? "";
      inFence = true;
      fenceChar = opening[2][0];
      fenceLength = opening[2].length;
      specialFenceLineIndex = STREAMING_SPECIAL_FENCE_LANGUAGES.has(language)
        ? i
        : -1;
      continue;
    }

    const closingPattern = new RegExp(
      `^ {0,3}${fenceChar}{${fenceLength},}\\s*$`,
    );
    if (closingPattern.test(line)) {
      inFence = false;
      specialFenceLineIndex = -1;
    }
  }

  if (!inFence || specialFenceLineIndex < 0) return text;

  lines[specialFenceLineIndex] = lines[specialFenceLineIndex].replace(
    /^( {0,3})(`{3,}|~{3,})([^\r\n]*)(\r?\n?)$/,
    `$1$2${STREAMING_SAFE_FENCE_LANGUAGE}$4`,
  );

  return lines.join("");
}

const PuzzleLink: React.FC<{
  children: string;
}> = ({ children }) => {
  const { handleLinkAction } = useLinksFromLsp();
  const link = extractLinkFromPuzzle(children);

  if (!link) return children;

  return (
    <div className={styles.puzzle_link}>
      <ChatLinkButton link={link} onClick={handleLinkAction} />
    </div>
  );
};

const MaybeInteractiveElement: React.FC<{
  key?: Key | null;
  children?: React.ReactNode;
}> = ({ children }) => {
  const processed = React.Children.map(children, (child, index) => {
    if (typeof child === "string" && child.startsWith("🧩")) {
      const key = `puzzle-link-${index}`;
      return <PuzzleLink key={key}>{child}</PuzzleLink>;
    }
    return child;
  });

  return <div className={styles.maybe_pin}>{processed}</div>;
};

const _Markdown: React.FC<MarkdownProps> = ({
  children,
  allowedElements,
  unwrapDisallowed,
  canHaveInteractiveElements,
  color,
  showLineNumbers,
  wrap,
  variant = "chat",
  onCopyClick,
  isStreaming,
}) => {
  const internalLinkContext = useInternalLinkHandler();

  const components: Partial<Components> = useMemo(() => {
    return {
      ol(props) {
        return (
          <ol {...props} className={classNames(styles.list, props.className)} />
        );
      },
      ul(props) {
        return (
          <ul {...props} className={classNames(styles.list, props.className)} />
        );
      },
      li({ color: _color, ref: _ref, node: _node, ...props }) {
        return (
          <li
            {...props}
            className={classNames(styles.list_item, props.className)}
          />
        );
      },
      code({ style: _style, color: _color, ...props }) {
        return (
          <ShikiCodeBlock
            color={color}
            showLineNumbers={showLineNumbers}
            wrap={wrap}
            onCopyClick={onCopyClick}
            isStreaming={isStreaming}
            {...props}
          />
        );
      },
      p({ color: _color, ref: _ref, node: _node, ...props }) {
        if (canHaveInteractiveElements) {
          return <MaybeInteractiveElement {...props} />;
        }
        return <p {...props} />;
      },
      h1({ color: _color, ref: _ref, node: _node, ...props }) {
        return <h1 {...props} />;
      },
      h2({ color: _color, ref: _ref, node: _node, ...props }) {
        return <h2 {...props} />;
      },
      h3({ color: _color, ref: _ref, node: _node, ...props }) {
        return <h3 {...props} />;
      },
      h4({ color: _color, ref: _ref, node: _node, ...props }) {
        return <h4 {...props} />;
      },
      h5({ color: _color, ref: _ref, node: _node, ...props }) {
        return <h5 {...props} />;
      },
      h6({ color: _color, ref: _ref, node: _node, ...props }) {
        return <h6 {...props} />;
      },
      blockquote({ color: _color, ref: _ref, node: _node, ...props }) {
        return <blockquote {...props} />;
      },
      em({ color: _color, ref: _ref, node: _node, ...props }) {
        return <em {...props} />;
      },
      kbd({ color: _color, ref: _ref, node: _node, ...props }) {
        return <kbd {...props} />;
      },
      a({ color: _color, ref: _ref, node: _node, ...props }) {
        const href = props.href ?? "";
        const isInternalLink = href.startsWith("refact://");
        const isHttpLink =
          href.startsWith("http://") || href.startsWith("https://");
        const isMailtoLink = href.startsWith("mailto:");
        const isSafeProtocol = isInternalLink || isHttpLink || isMailtoLink;

        if (!isSafeProtocol && href.includes(":")) {
          return <span>{props.children}</span>;
        }

        if (isInternalLink) {
          return (
            <Link
              {...props}
              href={href}
              onClick={(e: React.MouseEvent) => {
                if (internalLinkContext?.handleInternalLink(href)) {
                  e.preventDefault();
                }
              }}
              style={{ cursor: "pointer" }}
            />
          );
        }

        return (
          <Link
            {...props}
            target={isHttpLink ? "_blank" : undefined}
            rel={isHttpLink ? "noopener noreferrer" : undefined}
          />
        );
      },
      q({ color: _color, ref: _ref, node: _node, ...props }) {
        return <q {...props} />;
      },
      strong({ color: _color, ref: _ref, node: _node, ...props }) {
        return <strong {...props} />;
      },
      b({ color: _color, ref: _ref, node: _node, ...props }) {
        return (
          <span
            {...props}
            className={classNames(styles.bold, props.className)}
          />
        );
      },
      i({ color: _color, ref: _ref, node: _node, ...props }) {
        return <em {...props} />;
      },
      table({ color: _color, ref: _ref, node: _node, ...props }) {
        return (
          <table
            {...props}
            className={classNames(styles.table, props.className)}
          />
        );
      },
      tbody({ color: _color, ref: _ref, node: _node, ...props }) {
        return <tbody {...props} />;
      },
      thead({ color: _color, ref: _ref, node: _node, ...props }) {
        return <thead {...props} />;
      },
      tr({ color: _color, ref: _ref, node: _node, ...props }) {
        return <tr {...props} />;
      },
      th({ color: _color, ref: _ref, node: _node, ...props }) {
        return <th {...props} />;
      },
      td({ color: _color, ref: _ref, node: _node, width: _width, ...props }) {
        return <td {...props} />;
      },
    };
  }, [
    canHaveInteractiveElements,
    color,
    internalLinkContext,
    showLineNumbers,
    wrap,
    onCopyClick,
    isStreaming,
  ]);
  const renderedChildren =
    isStreaming && typeof children === "string"
      ? maskIncompleteSpecialCodeFences(children)
      : children;

  return (
    <ReactMarkdown
      className={classNames(styles.markdown, styles[`variant_${variant}`])}
      remarkPlugins={REMARK_PLUGINS}
      rehypePlugins={REHYPE_PLUGINS}
      urlTransform={transformMarkdownUrl}
      allowedElements={allowedElements}
      unwrapDisallowed={unwrapDisallowed}
      components={components}
    >
      {renderedChildren}
    </ReactMarkdown>
  );
};

export const Markdown = React.memo(_Markdown);

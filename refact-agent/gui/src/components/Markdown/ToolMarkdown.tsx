import React, { useMemo } from "react";
import ReactMarkdown, { Components } from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import { ShikiCodeBlock, type ShikiCodeBlockProps } from "./ShikiCodeBlock";
import toolStyles from "./ToolMarkdown.module.css";
import "katex/dist/katex.min.css";
import classNames from "classnames";

export type ToolMarkdownProps = Pick<
  React.ComponentProps<typeof ReactMarkdown>,
  "children" | "allowedElements" | "unwrapDisallowed"
> &
  Pick<ShikiCodeBlockProps, "color"> & {
    variant?: "chat" | "tool" | "terminal";
  };

/**
 * ToolMarkdown - A specialized markdown renderer for tool outputs
 *
 * Key differences from regular Markdown:
 * - All text renders at consistent size (terminal-like)
 * - Headings are bold but NOT larger (no scaling)
 * - Uses plain HTML elements with CSS styling (no Radix Text)
 * - Designed to match MarkdownCodeBlock visual style exactly
 */
export const ToolMarkdown: React.FC<ToolMarkdownProps> = ({
  children,
  allowedElements,
  unwrapDisallowed,
  color,
  variant = "tool",
}) => {
  const components: Partial<Components> = useMemo(() => {
    return {
      // Paragraphs
      p({ color: _color, ref: _ref, node: _node, ...props }) {
        return <p className={toolStyles.paragraph} {...props} />;
      },

      // Headings - all same size, just bold
      h1({ color: _color, ref: _ref, node: _node, ...props }) {
        return <div className={toolStyles.heading} {...props} />;
      },
      h2({ color: _color, ref: _ref, node: _node, ...props }) {
        return <div className={toolStyles.heading} {...props} />;
      },
      h3({ color: _color, ref: _ref, node: _node, ...props }) {
        return <div className={toolStyles.heading} {...props} />;
      },
      h4({ color: _color, ref: _ref, node: _node, ...props }) {
        return <div className={toolStyles.heading} {...props} />;
      },
      h5({ color: _color, ref: _ref, node: _node, ...props }) {
        return <div className={toolStyles.heading} {...props} />;
      },
      h6({ color: _color, ref: _ref, node: _node, ...props }) {
        return <div className={toolStyles.heading} {...props} />;
      },

      // Lists
      ol(props) {
        return <ol className={toolStyles.list} {...props} />;
      },
      ul(props) {
        return <ul className={toolStyles.list} {...props} />;
      },
      li({ color: _color, ref: _ref, node: _node, ...props }) {
        return <li className={toolStyles.listItem} {...props} />;
      },

      code({ style: _style, color: _color, ...props }) {
        return <ShikiCodeBlock color={color} {...props} />;
      },

      // Inline elements
      blockquote({ color: _color, ref: _ref, node: _node, ...props }) {
        return <blockquote className={toolStyles.blockquote} {...props} />;
      },
      em({ color: _color, ref: _ref, node: _node, ...props }) {
        return <em {...props} />;
      },
      strong({ color: _color, ref: _ref, node: _node, ...props }) {
        return <strong {...props} />;
      },
      b({ color: _color, ref: _ref, node: _node, ...props }) {
        return <strong {...props} />;
      },
      i({ color: _color, ref: _ref, node: _node, ...props }) {
        return <em {...props} />;
      },
      a({ color: _color, ref: _ref, node: _node, ...props }) {
        const href = props.href ?? "";
        const isHttpLink =
          href.startsWith("http://") || href.startsWith("https://");
        const isMailtoLink = href.startsWith("mailto:");
        const isSafeProtocol = isHttpLink || isMailtoLink;

        if (!isSafeProtocol && href.includes(":")) {
          return <span>{props.children}</span>;
        }

        return (
          <a
            className={toolStyles.link}
            {...props}
            target={isHttpLink ? "_blank" : undefined}
            rel={isHttpLink ? "noopener noreferrer" : undefined}
          />
        );
      },

      // Tables
      table({ color: _color, ref: _ref, node: _node, ...props }) {
        return <table className={toolStyles.table} {...props} />;
      },
      tbody({ color: _color, ref: _ref, node: _node, ...props }) {
        return <tbody {...props} />;
      },
      thead({ color: _color, ref: _ref, node: _node, ...props }) {
        return <thead className={toolStyles.thead} {...props} />;
      },
      tr({ color: _color, ref: _ref, node: _node, ...props }) {
        return <tr {...props} />;
      },
      th({ color: _color, ref: _ref, node: _node, ...props }) {
        return <th className={toolStyles.th} {...props} />;
      },
      td({ color: _color, ref: _ref, node: _node, width: _width, ...props }) {
        return <td className={toolStyles.td} {...props} />;
      },
    };
  }, [color]);

  return (
    <ReactMarkdown
      className={classNames(toolStyles.root, toolStyles[`variant_${variant}`])}
      remarkPlugins={[remarkBreaks, remarkMath, remarkGfm]}
      rehypePlugins={[rehypeKatex]}
      allowedElements={allowedElements}
      unwrapDisallowed={unwrapDisallowed}
      components={components}
    >
      {children}
    </ReactMarkdown>
  );
};

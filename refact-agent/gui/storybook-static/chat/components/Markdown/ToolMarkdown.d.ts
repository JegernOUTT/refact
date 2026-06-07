import React from "react";
import ReactMarkdown from "react-markdown";
import { type ShikiCodeBlockProps } from "./ShikiCodeBlock";
export type ToolMarkdownProps = Pick<React.ComponentProps<typeof ReactMarkdown>, "children" | "allowedElements" | "unwrapDisallowed"> & Pick<ShikiCodeBlockProps, "color">;
/**
 * ToolMarkdown - A specialized markdown renderer for tool outputs
 *
 * Key differences from regular Markdown:
 * - All text renders at consistent size (terminal-like)
 * - Headings are bold but NOT larger (no scaling)
 * - Uses plain HTML elements with CSS styling (no Radix Text)
 * - Designed to match MarkdownCodeBlock visual style exactly
 */
export declare const ToolMarkdown: React.FC<ToolMarkdownProps>;

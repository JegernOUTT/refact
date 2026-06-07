import React from "react";
import ReactMarkdown from "react-markdown";
import { type ShikiCodeBlockProps, type MarkdownControls } from "./ShikiCodeBlock";
export type MarkdownProps = Pick<React.ComponentProps<typeof ReactMarkdown>, "children" | "allowedElements" | "unwrapDisallowed"> & Pick<ShikiCodeBlockProps, "showLineNumbers" | "color" | "isStreaming"> & {
    canHaveInteractiveElements?: boolean;
    wrap?: boolean;
} & Partial<MarkdownControls>;
export declare const Markdown: React.NamedExoticComponent<MarkdownProps>;

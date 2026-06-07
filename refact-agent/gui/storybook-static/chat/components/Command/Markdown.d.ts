import React from "react";
export type MarkdownProps = {
    children: string;
    className?: string;
    isInsideScrollArea?: boolean;
};
export declare const Markdown: React.FC<MarkdownProps>;
export type CommandMarkdownProps = MarkdownProps;
export declare const CommandMarkdown: React.FC<CommandMarkdownProps>;

import React from "react";
import type { StoryObj } from "@storybook/react";
import type { ChatMessages } from "../../services/refact";
import type { ChatThread } from "../../features/Chat/Thread";
declare const meta: {
    title: string;
    component: React.FC<{
        messages?: ChatMessages;
        thread?: ChatThread;
    }>;
    args: {
        messages: never[];
    };
};
export default meta;
type Story = StoryObj<typeof meta>;
export declare const Primary: Story;
export declare const WithFunctions: Story;
export declare const Notes: Story;
export declare const WithDiffs: Story;
export declare const WithDiffActions: Story;
export declare const LargeDiff: Story;
export declare const Empty: Story;
export declare const AssistantMarkdown: Story;
export declare const ToolImages: Story;
export declare const MultiModal: Story;
export declare const IntegrationChat: Story;
export declare const TextDoc: Story;
export declare const MarkdownIssue: Story;
export declare const ToolWaiting: Story;

import React from "react";
import type { Meta, StoryObj } from "@storybook/react";
import { Usage } from "../../services/refact";
declare const MockedStore: React.FC<{
    usage: Usage;
    isInline?: boolean;
    isMessageEmpty?: boolean;
    threadMaximumContextTokens?: number;
    currentMessageContextTokens?: number;
}>;
declare const meta: Meta<typeof MockedStore>;
export default meta;
export declare const GPTUsageCounter: StoryObj<typeof MockedStore>;
export declare const AnthropicUsageCounter: StoryObj<typeof MockedStore>;
export declare const InlineUsageCounterInChatForm: StoryObj<typeof MockedStore>;

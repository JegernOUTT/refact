import React from "react";
import type { Meta, StoryObj } from "@storybook/react";
import { ChatThread } from "../../features/Chat/Thread/types";
import { RootState } from "../../app/store";
declare const Template: React.FC<{
    thread?: ChatThread;
    config?: RootState["config"];
}>;
declare const meta: Meta<typeof Template>;
export default meta;
type Story = StoryObj<typeof Template>;
export declare const Primary: Story;
export declare const Configuration: Story;
export declare const IDE: Story;
export declare const Knowledge: Story;
export declare const EmptySpaceAtBottom: Story;
export declare const UserMessageEmptySpaceAtBottom: Story;
export declare const CompressButton: Story;

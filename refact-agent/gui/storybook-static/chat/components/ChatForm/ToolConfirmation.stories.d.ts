import React from "react";
import type { Meta, StoryObj } from "@storybook/react";
import { ToolConfirmationPauseReason } from "../../services/refact";
declare const MockedStore: React.FC<{
    pauseReasons: ToolConfirmationPauseReason[];
}>;
declare const meta: Meta<typeof MockedStore>;
export default meta;
type Story = StoryObj<typeof meta>;
export declare const Default: Story;
export declare const WithDenial: Story;
export declare const Patch: Story;
export declare const Mixed: Story;

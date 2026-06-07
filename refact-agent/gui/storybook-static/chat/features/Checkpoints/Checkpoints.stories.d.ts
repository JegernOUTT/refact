import React from "react";
import type { StoryObj } from "@storybook/react";
import { CheckpointsMeta } from "./checkpointsSlice";
declare const Template: React.FC<{
    initialState?: CheckpointsMeta;
}>;
declare const meta: {
    title: string;
    component: React.FC<{
        initialState?: CheckpointsMeta;
    }>;
    parameters: {
        layout: string;
    };
};
export default meta;
type Story = StoryObj<typeof Template>;
export declare const Default: Story;
export declare const WithNoChanges: Story;
export declare const DialogClosed: Story;

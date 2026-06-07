import { ScrollAreaProps } from './ScrollArea';
import React from "react";
import type { StoryObj } from "@storybook/react";
declare const meta: {
    title: string;
    component: React.ForwardRefExoticComponent<Omit<ScrollAreaProps, "ref"> & React.RefAttributes<HTMLDivElement>>;
    args: {
        scrollbars: "vertical";
        style: {
            height: string;
        };
    };
};
export default meta;
type Story = StoryObj<typeof meta>;
export declare const Primary: Story;

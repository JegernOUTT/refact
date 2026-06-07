import { JSX } from 'react/jsx-runtime';
import type { StoryObj } from "@storybook/react";
declare function DesignSystemOverview(): JSX.Element;
declare const meta: {
    title: string;
    component: typeof DesignSystemOverview;
    parameters: {
        layout: string;
    };
};
export default meta;
type Story = StoryObj<typeof meta>;
export declare const Overview: Story;

import { TextAreaProps } from './TextArea';
import { ForwardRefExoticComponent, RefAttributes } from 'react';
import type { StoryObj } from "@storybook/react";
declare const meta: {
    title: string;
    component: ForwardRefExoticComponent<Omit<TextAreaProps, "ref"> & RefAttributes<HTMLTextAreaElement>>;
    args: {};
};
export default meta;
type Story = StoryObj<typeof meta>;
export declare const Primary: Story;

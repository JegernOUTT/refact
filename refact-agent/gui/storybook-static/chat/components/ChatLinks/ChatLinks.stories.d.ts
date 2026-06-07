import { JSX } from 'react/jsx-runtime';
import { StoryObj } from "@storybook/react";
import { type HttpHandler } from "msw";
declare const meta: {
    title: string;
    component: () => JSX.Element;
    argTypes: {};
    parameters: {
        msw: {
            handlers: HttpHandler[];
        };
    };
};
export default meta;
type Story = StoryObj<typeof meta>;
export declare const Default: Story;

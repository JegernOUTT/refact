import { CalloutProps } from './Callout';
import { FC } from 'react';
import type { StoryObj } from "@storybook/react";
import { ErrorCallout } from ".";
declare const meta: {
    title: string;
    component: FC<Omit<CalloutProps, "type">>;
};
export default meta;
export declare const Default: StoryObj<typeof ErrorCallout>;

import { JSX } from 'react/jsx-runtime';
import type { Meta, StoryObj } from "@storybook/react";
declare const App: () => JSX.Element;
declare const meta: Meta<typeof App>;
export default meta;
type Story = StoryObj<typeof meta>;
export declare const Primary: Story;

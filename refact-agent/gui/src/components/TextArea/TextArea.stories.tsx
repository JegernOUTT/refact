import type { Meta, StoryObj } from "@storybook/react";
import { TextArea } from "./TextArea";

const meta = {
  title: "TextArea",
  component: TextArea,
  // SB8 no longer auto-creates implicit action args, and TextArea calls this
  // during the layout effect on first render (ImplicitActionsDuringRendering).
  args: {
    onTextAreaHeightChange: () => undefined,
  },
} satisfies Meta<typeof TextArea>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Primary: Story = {};

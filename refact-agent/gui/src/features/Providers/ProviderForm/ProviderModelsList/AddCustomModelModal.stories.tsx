import type { Meta, StoryObj } from "@storybook/react";
import { fn } from "@storybook/test";
import { Provider } from "react-redux";

import { setUpStore } from "../../../../app/store";
import { AddCustomModelModal } from "./AddCustomModelModal";

const meta = {
  title: "Features/Providers/AddCustomModelModal",
  component: AddCustomModelModal,
  decorators: [
    (Story) => (
      <Provider store={setUpStore()}>
        <Story />
      </Provider>
    ),
  ],
  args: {
    providerName: "openai_work",
    isOpen: true,
    onClose: fn(),
  },
} satisfies Meta<typeof AddCustomModelModal>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Open: Story = {};

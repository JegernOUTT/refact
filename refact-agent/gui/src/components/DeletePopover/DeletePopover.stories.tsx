import type { Meta, StoryObj } from "@storybook/react";
import { Provider } from "react-redux";
import { setUpStore } from "../../app/store";
import { within, userEvent } from "@storybook/testing-library";
import { fn } from "@storybook/test";
import { DeletePopover } from "./DeletePopover";

const meta = {
  title: "Components/DeletePopover",
  component: DeletePopover,
  decorators: [
    (Story) => (
      <Provider store={setUpStore()}>
        <Story />
      </Provider>
    ),
  ],
  args: {
    isDisabled: false,
    isDeleting: false,
    itemName: "GitHub MCP server",
    deleteBy: "github",
    handleDelete: fn(),
  },
} satisfies Meta<typeof DeletePopover>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Closed: Story = {};

export const Open: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(
      canvas.getByRole("button", { name: "Delete GitHub MCP server" }),
    );
  },
};

export const Small: Story = {
  args: { size: "sm" },
  play: Open.play,
};

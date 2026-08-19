import type { Meta, StoryObj } from "@storybook/react";
import { fn } from "@storybook/test";
import { Provider } from "react-redux";
import { setUpStore } from "../../app/store";
import { CreateWorktreeModal } from "./CreateWorktreeModal";

const meta = {
  title: "Features/Worktrees/CreateWorktreeModal",
  component: CreateWorktreeModal,
  decorators: [
    (Story) => (
      <Provider store={setUpStore()}>
        <Story />
      </Provider>
    ),
  ],
  args: {
    open: true,
    defaultBranch: "feature/storybook-dialogs",
    defaultBaseBranch: "main",
    baseBranchOptions: ["main", "develop", "release/next"],
    isCreating: false,
    onOpenChange: fn(),
    onCreate: fn(() => Promise.resolve()),
  },
} satisfies Meta<typeof CreateWorktreeModal>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Open: Story = {};

export const WithError: Story = {
  args: {
    error: "A worktree for this branch already exists.",
  },
};

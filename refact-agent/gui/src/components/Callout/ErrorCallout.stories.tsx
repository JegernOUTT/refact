import type { Meta, StoryObj } from "@storybook/react";
import { Provider } from "react-redux";
import { setUpStore } from "../../app/store";
import { ErrorCallout } from ".";

const meta = {
  title: "Error Callout",
  component: ErrorCallout,
  decorators: [
    (Story) => (
      <Provider store={setUpStore()}>
        <Story />
      </Provider>
    ),
  ],
} satisfies Meta<typeof ErrorCallout>;

export default meta;

export const Default: StoryObj<typeof ErrorCallout> = {
  args: {
    children: "some bad happened",
  },
};

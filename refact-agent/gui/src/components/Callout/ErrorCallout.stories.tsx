import type { Meta, StoryObj } from "@storybook/react";
import { ErrorCalloutView } from ".";

const meta = {
  title: "Error Callout",
  component: ErrorCalloutView,
  args: {
    isAuthError: false,
  },
} satisfies Meta<typeof ErrorCalloutView>;

export default meta;

export const Default: StoryObj<typeof ErrorCalloutView> = {
  args: {
    children: "some bad happened",
  },
};

export const AuthError: StoryObj<typeof ErrorCalloutView> = {
  args: {
    children: "unauthorized",
    isAuthError: true,
  },
};

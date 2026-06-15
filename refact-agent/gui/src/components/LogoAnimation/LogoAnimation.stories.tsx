import React from "react";
import type { Meta, StoryObj } from "@storybook/react";
import { LogoAnimation } from "./LogoAnimation";
import { Theme } from "../Theme";
import { Provider } from "react-redux";
import { setUpStore } from "../../app/store";
import { Surface } from "../ui";

const App: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const store = setUpStore();
  return (
    <Provider store={store}>
      <Theme>
        <div style={{ padding: "var(--rf-space-6)" }}>
          <Surface variant="surface-1" radius="card">
            {children}
          </Surface>
        </div>
      </Theme>
    </Provider>
  );
};
const meta: Meta<typeof LogoAnimation> = {
  title: "Logo Animation",
  component: LogoAnimation,
  decorators: [
    (Story) => (
      <App>
        <Story />
      </App>
    ),
  ],
};

export default meta;

type Story = StoryObj<typeof LogoAnimation>;

export const Streaming: Story = {
  args: { isStreaming: true, isWaiting: false },
};
export const Waiting: Story = { args: { isStreaming: false, isWaiting: true } };
export const Idle: Story = { args: { isStreaming: false, isWaiting: false } };

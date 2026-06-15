import type { Meta, StoryObj } from "@storybook/react";

import { Menu } from "./Menu";
import styles from "../Overlay.stories.module.css";

function MenuStory({ reducedMotion = false }: { reducedMotion?: boolean }) {
  return (
    <div
      className={`${styles.storyShell} ${
        reducedMotion ? styles.reducedMotion : ""
      }`}
    >
      {(["light", "dark"] as const).map((appearance) => (
        <section
          className={styles.panel}
          data-appearance={appearance}
          key={appearance}
        >
          <div className={styles.header}>
            <h2 className={styles.title}>{appearance} menu</h2>
            <p className={styles.description}>
              Panel-less DropdownMenu rows with subtle hover, label, item, and
              separator slots.
            </p>
          </div>
          <Menu defaultOpen>
            <Menu.Trigger asChild>
              <button className={styles.button} type="button">
                Open menu
              </button>
            </Menu.Trigger>
            <Menu.Content maxHeight="320px">
              <Menu.Label>Session</Menu.Label>
              <Menu.Item>New chat</Menu.Item>
              <Menu.Item>Rename thread</Menu.Item>
              <Menu.Separator />
              <Menu.Item>Copy transcript</Menu.Item>
              <Menu.Item disabled>Archive unavailable</Menu.Item>
            </Menu.Content>
          </Menu>
        </section>
      ))}
    </div>
  );
}

const meta = {
  title: "UI/Overlays/Menu",
  component: Menu,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Menu>;

export default meta;

type Story = StoryObj<typeof meta>;

export const LightDark: Story = { render: () => <MenuStory /> };
export const ReducedMotion: Story = {
  render: () => <MenuStory reducedMotion />,
};

import type { Meta, StoryObj } from "@storybook/react";
import { within, userEvent } from "@storybook/testing-library";

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
          // The light half must paint its own canvas and flip color-scheme,
          // otherwise it inherits the dark UA scheme and reads as token soup
          // (audit N-03).
          className={`${styles.panel} ${appearance === "light" ? "light" : ""}`}
          data-appearance={appearance}
          key={appearance}
          style={
            appearance === "light"
              ? { background: "var(--rf-bg)", colorScheme: "light" }
              : undefined
          }
        >
          <div className={styles.header}>
            <h2 className={styles.title}>{appearance} menu</h2>
            <p className={styles.description}>
              Panel-less DropdownMenu rows with subtle hover, label, item, and
              separator slots.
            </p>
          </div>
          {/* .row keeps the trigger at its intrinsic size; a bare button is a
              grid item of .panel and would stretch to the full panel width. */}
          <div className={styles.row}>
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
          </div>
        </section>
      ))}
    </div>
  );
}

// defaultOpen already opens the menu; this only covers the case where the
// overlay has been dismissed, so the story never renders trigger-only (N-16).
async function openFirstMenu(canvasElement: HTMLElement) {
  const canvas = within(canvasElement);
  const [trigger] = canvas.getAllByRole("button", { name: /open menu/i });
  if (trigger.getAttribute("aria-expanded") !== "true") {
    await userEvent.click(trigger);
  }
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
  // Pin the html attribute so the portaled overlay stops animating too.
  parameters: { reducedMotion: "on" },
  render: () => <MenuStory reducedMotion />,
};

// The menu is portaled to document.body, so a side-by-side light panel can
// never show it in light mode. parameters.appearance pins the whole preview
// (including the portal root) to light instead (audit N-04).
export const LightOverlay: Story = {
  parameters: { appearance: "light" },
  render: () => <MenuStory />,
  play: ({ canvasElement }) => openFirstMenu(canvasElement),
};

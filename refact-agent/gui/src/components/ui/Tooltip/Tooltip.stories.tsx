import type { Meta, StoryObj } from "@storybook/react";

import { Tooltip } from "./Tooltip";
import styles from "../Overlay.stories.module.css";

function TooltipStory({ reducedMotion = false }: { reducedMotion?: boolean }) {
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
            <h2 className={styles.title}>{appearance} tooltip</h2>
            <p className={styles.description}>
              Hover or focus the trigger to show a clamped tooltip.
            </p>
          </div>
          <Tooltip>
            <Tooltip.Trigger asChild>
              <button className={styles.button} type="button">
                Hover or focus me
              </button>
            </Tooltip.Trigger>
            <Tooltip.Content>
              Tooltip content wraps and clamps to the viewport while preserving
              theme tokens.
            </Tooltip.Content>
          </Tooltip>
        </section>
      ))}
    </div>
  );
}

const meta = {
  title: "UI/Overlays/Tooltip",
  component: Tooltip,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Tooltip>;

export default meta;

type Story = StoryObj<typeof meta>;

export const LightDark: Story = { render: () => <TooltipStory /> };
export const ReducedMotion: Story = {
  render: () => <TooltipStory reducedMotion />,
};

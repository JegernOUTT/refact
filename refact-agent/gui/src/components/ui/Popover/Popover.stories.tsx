import type { Meta, StoryObj } from "@storybook/react";

import { Popover } from "./Popover";
import styles from "../Overlay.stories.module.css";

function PopoverContent() {
  return (
    <div className={styles.contentText}>
      <strong>Assistant options</strong>
      <span>Anchored panel with clamped dimensions, theme tokens, and Escape handling.</span>
      <div className="scrollX">
        <div className={styles.longBox}>Long popover content uses .scrollX for horizontal overflow.</div>
      </div>
    </div>
  );
}

function PopoverStory({ forceSheet = false, reducedMotion = false }: { forceSheet?: boolean; reducedMotion?: boolean }) {
  return (
    <div className={`${styles.storyShell} ${reducedMotion ? styles.reducedMotion : ""}`}>
      {(["light", "dark"] as const).map((appearance) => (
        <section className={`${styles.panel} ${forceSheet ? styles.narrowPanel : ""}`} data-appearance={appearance} key={appearance}>
          <div className={styles.header}>
            <h2 className={styles.title}>{appearance} popover</h2>
            <p className={styles.description}>Responsive popover becomes a Sheet below 480px; this story can force the sheet branch.</p>
          </div>
          <Popover forceSheet={forceSheet}>
            <Popover.Trigger asChild>
              <button className={styles.button} type="button">Open popover</button>
            </Popover.Trigger>
            <Popover.Content maxHeight="320px" maxWidth="360px">
              <PopoverContent />
            </Popover.Content>
          </Popover>
        </section>
      ))}
    </div>
  );
}

const meta = {
  title: "UI/Overlays/Popover",
  component: Popover,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Popover>;

export default meta;

type Story = StoryObj<typeof meta>;

export const LightDark: Story = { render: () => <PopoverStory /> };
export const NarrowSheet: Story = { render: () => <PopoverStory forceSheet /> };
export const ReducedMotion: Story = { render: () => <PopoverStory reducedMotion /> };

import type { Meta, StoryObj } from "@storybook/react";

import { Sheet } from "./Sheet";
import styles from "../Overlay.stories.module.css";

function SheetStory({ reducedMotion = false }: { reducedMotion?: boolean }) {
  return (
    <div
      className={`${styles.storyShell} ${
        reducedMotion ? styles.reducedMotion : ""
      }`}
    >
      {(["light", "dark"] as const).map((appearance) => (
        <section
          className={`${styles.panel} ${styles.narrowPanel}`}
          data-appearance={appearance}
          key={appearance}
        >
          <div className={styles.header}>
            <h2 className={styles.title}>{appearance} sheet</h2>
            <p className={styles.description}>
              Bottom sheet for narrow modal flows; side can be changed by prop.
            </p>
          </div>
          <Sheet>
            <Sheet.Trigger asChild>
              <button className={styles.button} type="button">
                Open sheet
              </button>
            </Sheet.Trigger>
            <Sheet.Content maxHeight="360px" side="bottom">
              <Sheet.Title>Mobile settings</Sheet.Title>
              <Sheet.Description>
                Edge-anchored panel with title and description wiring.
              </Sheet.Description>
              <div className={styles.longContent}>
                {Array.from({ length: 7 }, (_, index) => (
                  <p className={styles.description} key={index}>
                    Sheet row {index + 1} remains inside the clamped scroll
                    body.
                  </p>
                ))}
              </div>
              <div className={styles.actions}>
                <Sheet.Close asChild>
                  <button className={styles.button} type="button">
                    Close
                  </button>
                </Sheet.Close>
              </div>
            </Sheet.Content>
          </Sheet>
        </section>
      ))}
    </div>
  );
}

const meta = {
  title: "UI/Overlays/Sheet",
  component: Sheet,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Sheet>;

export default meta;

type Story = StoryObj<typeof meta>;

export const LightDark: Story = { render: () => <SheetStory /> };
export const Narrow: Story = { render: () => <SheetStory /> };
export const ReducedMotion: Story = {
  render: () => <SheetStory reducedMotion />,
};

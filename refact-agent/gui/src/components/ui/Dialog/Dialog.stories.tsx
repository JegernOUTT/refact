import type { Meta, StoryObj } from "@storybook/react";

import { Dialog } from "./Dialog";
import styles from "../Overlay.stories.module.css";

function DialogStory({ reducedMotion = false }: { reducedMotion?: boolean }) {
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
            <h2 className={styles.title}>{appearance} dialog</h2>
            <p className={styles.description}>
              Centered modal with title, description, focus trap, Escape, and
              scrollable body.
            </p>
          </div>
          <Dialog>
            <Dialog.Trigger asChild>
              <button className={styles.button} type="button">
                Open dialog
              </button>
            </Dialog.Trigger>
            <Dialog.Content maxHeight="360px">
              <Dialog.Title>Confirm model change</Dialog.Title>
              <Dialog.Description>
                This dialog is rendered through the theme-wrapped Portal.
              </Dialog.Description>
              <div className={styles.longContent}>
                {Array.from({ length: 8 }, (_, index) => (
                  <p className={styles.description} key={index}>
                    Dialog body row {index + 1} demonstrates vertical scrolling
                    inside the clamped overlay.
                  </p>
                ))}
                <div className="scrollX">
                  <div className={styles.longBox}>
                    Wide content stays in an explicit horizontal scroll island.
                  </div>
                </div>
              </div>
              <div className={styles.actions}>
                <Dialog.Close asChild>
                  <button className={styles.button} type="button">
                    Close
                  </button>
                </Dialog.Close>
              </div>
            </Dialog.Content>
          </Dialog>
        </section>
      ))}
    </div>
  );
}

const meta = {
  title: "UI/Overlays/Dialog",
  component: Dialog,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Dialog>;

export default meta;

type Story = StoryObj<typeof meta>;

export const LightDark: Story = { render: () => <DialogStory /> };
export const ReducedMotion: Story = {
  render: () => <DialogStory reducedMotion />,
};

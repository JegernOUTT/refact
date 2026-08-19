import type { Meta, StoryObj } from "@storybook/react";
import { within, userEvent } from "@storybook/testing-library";

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
          // The light half must paint its own canvas and flip color-scheme,
          // otherwise it inherits the dark UA scheme and reads as token soup
          // (audit N-03).
          className={`${styles.panel} ${appearance === "light" ? "light" : ""}`}
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
          {/* .row keeps the trigger at its intrinsic size; a bare button is a
              grid item of .panel and would stretch to the full panel width. */}
          <div className={styles.row}>
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
                      Dialog body row {index + 1} demonstrates vertical
                      scrolling inside the clamped overlay.
                    </p>
                  ))}
                  <div className="scrollX">
                    <div className={styles.longBox}>
                      Wide content stays in an explicit horizontal scroll
                      island.
                    </div>
                  </div>
                </div>
                <Dialog.Footer>
                  <Dialog.Close asChild>
                    <button className={styles.button} type="button">
                      Close
                    </button>
                  </Dialog.Close>
                </Dialog.Footer>
              </Dialog.Content>
            </Dialog>
          </div>
        </section>
      ))}
    </div>
  );
}

// Trigger-only overlay stories shipped closed, so the overlay itself was never
// reviewed or snapshotted (audit N-16).
async function openFirstDialog(canvasElement: HTMLElement) {
  const canvas = within(canvasElement);
  const [trigger] = canvas.getAllByRole("button", { name: /open dialog/i });
  if (trigger.getAttribute("aria-expanded") !== "true") {
    await userEvent.click(trigger);
  }
}

const meta = {
  title: "UI/Overlays/Dialog",
  component: Dialog,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Dialog>;

export default meta;

type Story = StoryObj<typeof meta>;

export const LightDark: Story = {
  render: () => <DialogStory />,
  play: ({ canvasElement }) => openFirstDialog(canvasElement),
};
export const ReducedMotion: Story = {
  // Pin the html attribute so the portaled overlay stops animating too.
  parameters: { reducedMotion: "on" },
  render: () => <DialogStory reducedMotion />,
  play: ({ canvasElement }) => openFirstDialog(canvasElement),
};

// The dialog is portaled to document.body, so a side-by-side light panel can
// never show it in light mode. parameters.appearance pins the whole preview
// (including the portal root) to light instead (audit N-04).
export const LightOverlay: Story = {
  parameters: { appearance: "light" },
  render: () => <DialogStory />,
  play: ({ canvasElement }) => openFirstDialog(canvasElement),
};

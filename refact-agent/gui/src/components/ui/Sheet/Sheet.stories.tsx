import type { Meta, StoryObj } from "@storybook/react";
import { within, userEvent } from "@storybook/testing-library";

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
          // The light half must paint its own canvas and flip color-scheme,
          // otherwise it inherits the dark UA scheme and reads as token soup
          // (audit N-03).
          className={`${styles.panel} ${styles.narrowPanel} ${
            appearance === "light" ? "light" : ""
          }`}
          data-appearance={appearance}
          key={appearance}
          style={
            appearance === "light"
              ? { background: "var(--rf-bg)", colorScheme: "light" }
              : undefined
          }
        >
          <div className={styles.header}>
            <h2 className={styles.title}>{appearance} sheet</h2>
            <p className={styles.description}>
              Bottom sheet for narrow modal flows; side can be changed by prop.
            </p>
          </div>
          {/* .row keeps the trigger at its intrinsic size; a bare button is a
              grid item of .panel and would stretch to the full panel width. */}
          <div className={styles.row}>
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
          </div>
        </section>
      ))}
    </div>
  );
}

// Trigger-only overlay stories shipped closed, so the overlay itself was never
// reviewed or snapshotted (audit N-16).
async function openFirstSheet(canvasElement: HTMLElement) {
  const canvas = within(canvasElement);
  const [trigger] = canvas.getAllByRole("button", { name: /open sheet/i });
  if (trigger.getAttribute("aria-expanded") !== "true") {
    await userEvent.click(trigger);
  }
}

const meta = {
  title: "UI/Overlays/Sheet",
  component: Sheet,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Sheet>;

export default meta;

type Story = StoryObj<typeof meta>;

export const LightDark: Story = {
  render: () => <SheetStory />,
  play: ({ canvasElement }) => openFirstSheet(canvasElement),
};
export const Narrow: Story = {
  render: () => <SheetStory />,
  play: ({ canvasElement }) => openFirstSheet(canvasElement),
};
export const ReducedMotion: Story = {
  // Pin the html attribute so the portaled overlay stops animating too.
  parameters: { reducedMotion: "on" },
  render: () => <SheetStory reducedMotion />,
  play: ({ canvasElement }) => openFirstSheet(canvasElement),
};

// The sheet is portaled to document.body, so a side-by-side light panel can
// never show it in light mode. parameters.appearance pins the whole preview
// (including the portal root) to light instead (audit N-04).
export const LightOverlay: Story = {
  parameters: { appearance: "light" },
  render: () => <SheetStory />,
  play: ({ canvasElement }) => openFirstSheet(canvasElement),
};

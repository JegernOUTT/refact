import type { Meta, StoryObj } from "@storybook/react";
import { within, userEvent } from "@storybook/testing-library";

import { Popover } from "./Popover";
import styles from "../Overlay.stories.module.css";

function PopoverContent() {
  return (
    <div className={styles.contentText}>
      <strong>Assistant options</strong>
      <span>
        Anchored panel with clamped dimensions, theme tokens, and Escape
        handling.
      </span>
      <div className="scrollX">
        <div className={styles.longBox}>
          Long popover content uses .scrollX for horizontal overflow.
        </div>
      </div>
    </div>
  );
}

function PopoverStory({
  forceSheet = false,
  reducedMotion = false,
}: {
  forceSheet?: boolean;
  reducedMotion?: boolean;
}) {
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
          className={`${styles.panel} ${forceSheet ? styles.narrowPanel : ""} ${
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
            <h2 className={styles.title}>{appearance} popover</h2>
            <p className={styles.description}>
              Responsive popover becomes a Sheet below 480px; this story can
              force the sheet branch.
            </p>
          </div>
          {/* .row keeps the trigger at its intrinsic size; a bare button is a
              grid item of .panel and would stretch to the full panel width. */}
          <div className={styles.row}>
            <Popover forceSheet={forceSheet}>
              <Popover.Trigger asChild>
                <button className={styles.button} type="button">
                  Open popover
                </button>
              </Popover.Trigger>
              <Popover.Content maxHeight="320px" maxWidth="360px">
                <PopoverContent />
              </Popover.Content>
            </Popover>
          </div>
        </section>
      ))}
    </div>
  );
}

// Trigger-only overlay stories shipped closed, so the overlay itself was never
// reviewed or snapshotted (audit N-16).
async function openFirstPopover(canvasElement: HTMLElement) {
  const canvas = within(canvasElement);
  const [trigger] = canvas.getAllByRole("button", { name: /open popover/i });
  if (trigger.getAttribute("aria-expanded") !== "true") {
    await userEvent.click(trigger);
  }
}

const meta = {
  title: "UI/Overlays/Popover",
  component: Popover,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Popover>;

export default meta;

type Story = StoryObj<typeof meta>;

export const LightDark: Story = {
  render: () => <PopoverStory />,
  play: ({ canvasElement }) => openFirstPopover(canvasElement),
};
export const NarrowSheet: Story = {
  render: () => <PopoverStory forceSheet />,
  play: ({ canvasElement }) => openFirstPopover(canvasElement),
};
export const ReducedMotion: Story = {
  // Pin the html attribute so the portaled overlay stops animating too.
  parameters: { reducedMotion: "on" },
  render: () => <PopoverStory reducedMotion />,
  play: ({ canvasElement }) => openFirstPopover(canvasElement),
};

// The popover is portaled to document.body, so a side-by-side light panel can
// never show it in light mode. parameters.appearance pins the whole preview
// (including the portal root) to light instead (audit N-04).
export const LightOverlay: Story = {
  parameters: { appearance: "light" },
  render: () => <PopoverStory />,
  play: ({ canvasElement }) => openFirstPopover(canvasElement),
};

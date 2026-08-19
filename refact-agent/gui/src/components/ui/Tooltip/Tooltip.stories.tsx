import type { Meta, StoryObj } from "@storybook/react";
import { within, userEvent, screen } from "@storybook/testing-library";

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
            <h2 className={styles.title}>{appearance} tooltip</h2>
            <p className={styles.description}>
              Hover or focus the trigger to show a clamped tooltip.
            </p>
          </div>
          {/* .row keeps the trigger at its intrinsic size; a bare button is a
              grid item of .panel and would stretch to the full panel width. */}
          <div className={styles.row}>
            <Tooltip>
              <Tooltip.Trigger asChild>
                <button className={styles.button} type="button">
                  Hover or focus me
                </button>
              </Tooltip.Trigger>
              <Tooltip.Content>
                Tooltip content wraps and clamps to the viewport while
                preserving theme tokens.
              </Tooltip.Content>
            </Tooltip>
          </div>
        </section>
      ))}
    </div>
  );
}

// Trigger-only overlay stories shipped closed, so the tooltip surface was never
// reviewed or snapshotted (audit N-16). Focus opens Radix tooltips immediately,
// hover honours the open delay - do both so either path wins.
async function showFirstTooltip(canvasElement: HTMLElement) {
  const canvas = within(canvasElement);
  const [trigger] = canvas.getAllByRole("button", {
    name: /hover or focus me/i,
  });
  await userEvent.hover(trigger);
  trigger.focus();
  // The tooltip is portaled to document.body, so query the whole screen.
  // Never fail the story if the popper is still animating in.
  await screen.findByRole("tooltip").catch(() => null);
}

const meta = {
  title: "UI/Overlays/Tooltip",
  component: Tooltip,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Tooltip>;

export default meta;

type Story = StoryObj<typeof meta>;

export const LightDark: Story = {
  render: () => <TooltipStory />,
  play: ({ canvasElement }) => showFirstTooltip(canvasElement),
};
export const ReducedMotion: Story = {
  render: () => <TooltipStory reducedMotion />,
  play: ({ canvasElement }) => showFirstTooltip(canvasElement),
};

// The tooltip is portaled to document.body, so a side-by-side light panel can
// never show it in light mode. parameters.appearance pins the whole preview
// (including the portal root) to light instead (audit N-04).
export const LightOverlay: Story = {
  parameters: { appearance: "light" },
  render: () => <TooltipStory />,
  play: ({ canvasElement }) => showFirstTooltip(canvasElement),
};

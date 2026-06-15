import type { Meta, StoryObj } from "@storybook/react";
import { StatusDot, type StatusDotStatus } from ".";
import styles from "./StatusDot.stories.module.css";

const statuses: StatusDotStatus[] = [
  "idle",
  "running",
  "success",
  "error",
  "warning",
  "paused",
  "in_progress",
  "needs_attention",
  "completed",
];

function StatusDotGallery() {
  return (
    <main className={styles.gallery}>
      <h2 className={styles.title}>StatusDot states</h2>
      <div className={styles.grid}>
        {statuses.map((status) => (
          <div className={styles.item} key={status}>
            <StatusDot
              aria-label={status}
              pulse={status === "running" || status === "in_progress"}
              status={status}
            />
            <span>{status}</span>
          </div>
        ))}
      </div>
      <p className={styles.note}>
        Pulse uses .rf-status-pulse and is disabled by the reduced-motion media
        query from the shared motion utility.
      </p>
    </main>
  );
}

const meta = {
  title: "Design System/StatusDot",
  component: StatusDotGallery,
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta<typeof StatusDotGallery>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Gallery: Story = {};

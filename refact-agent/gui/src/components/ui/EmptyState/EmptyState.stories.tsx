import type { Meta, StoryObj } from "@storybook/react";
import { Inbox } from "lucide-react";
import { EmptyState } from ".";
import styles from "./EmptyState.stories.module.css";

function EmptyStateGallery() {
  return (
    <main className={styles.gallery}>
      <section className={styles.section}>
        <h2 className={styles.title}>Empty state</h2>
        <p className={styles.note}>
          Panel-less, centered, and muted for compact inline slots or full-page
          feature surfaces. Toggle Storybook light, dark, narrow, and
          reduced-motion modes for review.
        </p>
        <div className={styles.grid}>
          <div className={styles.sample}>
            <EmptyState
              description="Try changing filters or adding the first provider."
              icon={Inbox}
              title="No providers found"
            />
          </div>
          <div className={styles.sampleFull}>
            <EmptyState
              description="Once knowledge memories are indexed, they will appear here."
              icon={Inbox}
              title="Nothing here yet"
              variant="full"
            />
          </div>
        </div>
      </section>
    </main>
  );
}

const meta = {
  title: "Design System/EmptyState",
  component: EmptyStateGallery,
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta<typeof EmptyStateGallery>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Gallery: Story = {};

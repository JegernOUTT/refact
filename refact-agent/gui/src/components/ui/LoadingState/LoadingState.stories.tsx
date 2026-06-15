import type { Meta, StoryObj } from "@storybook/react";
import { LoadingState } from ".";
import styles from "./LoadingState.stories.module.css";

function LoadingStateGallery() {
  return (
    <main className={styles.gallery}>
      <section className={styles.section}>
        <h2 className={styles.title}>Loading state</h2>
        <p className={styles.note}>
          Spinner and skeleton variants support compact inline and full-page
          presentations. Skeleton shimmer comes from .rf-shimmer and honors
          reduced-motion.
        </p>
        <div className={styles.grid}>
          <div className={styles.sample}>
            <LoadingState label="Loading providers" />
          </div>
          <div className={styles.sample}>
            <LoadingState kind="skeleton" label="Loading rows" />
          </div>
          <div className={styles.sampleFull}>
            <LoadingState
              kind="skeleton"
              label="Preparing dashboard"
              variant="full"
            />
          </div>
        </div>
      </section>
    </main>
  );
}

const meta = {
  title: "Design System/LoadingState",
  component: LoadingStateGallery,
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta<typeof LoadingStateGallery>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Gallery: Story = {};

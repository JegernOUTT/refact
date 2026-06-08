import type { Meta, StoryObj } from "@storybook/react";
import { Skeleton, SkeletonText } from ".";
import styles from "./Skeleton.stories.module.css";

function SkeletonGallery() {
  return (
    <main className={styles.gallery}>
      <section className={styles.section}>
        <h2 className={styles.title}>Skeleton</h2>
        <p className={styles.note}>
          Skeleton composes the shared .rf-shimmer utility, so shimmer motion is
          transform/opacity-only and reduced-motion aware.
        </p>
        <div className={styles.grid}>
          <div className={styles.cardSkeleton}>
            <Skeleton height="96px" radius="card" />
            <SkeletonText lines={3} />
          </div>
          <div className={styles.listSkeleton}>
            {Array.from({ length: 4 }).map((_, index) => (
              <div className={styles.listRow} key={index}>
                <Skeleton
                  height="var(--rf-control-h)"
                  radius="pill"
                  width="var(--rf-control-h)"
                />
                <SkeletonText lines={2} />
              </div>
            ))}
          </div>
        </div>
      </section>
    </main>
  );
}

const meta = {
  title: "Design System/Skeleton",
  component: SkeletonGallery,
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta<typeof SkeletonGallery>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Gallery: Story = {};

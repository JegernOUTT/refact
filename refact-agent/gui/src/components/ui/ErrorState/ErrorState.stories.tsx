import type { Meta, StoryObj } from "@storybook/react";
import { ErrorState } from ".";
import styles from "./ErrorState.stories.module.css";

function RetryAction() {
  return <button className={styles.button}>Retry</button>;
}

function ErrorStateGallery() {
  return (
    <main className={styles.gallery}>
      <section className={styles.section}>
        <h2 className={styles.title}>Error state</h2>
        <p className={styles.note}>
          Danger tone is color-only, with compact inline and full-page layouts
          for failed provider, marketplace, knowledge, and stats surfaces.
        </p>
        <div className={styles.grid}>
          <div className={styles.sample}>
            <ErrorState
              description="Provider settings could not be loaded."
              retry={<RetryAction />}
              title="Could not load providers"
            />
          </div>
          <div className={styles.sampleFull}>
            <ErrorState
              error={
                new Error("The statistics service returned an empty response.")
              }
              retry={<RetryAction />}
              title="Stats unavailable"
              variant="full"
            />
          </div>
        </div>
      </section>
    </main>
  );
}

const meta = {
  title: "Design System/ErrorState",
  component: ErrorStateGallery,
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta<typeof ErrorStateGallery>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Gallery: Story = {};

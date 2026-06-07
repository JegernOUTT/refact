import type { Meta, StoryObj } from "@storybook/react";
import { Surface, type SurfaceVariant } from ".";
import styles from "./Surface.stories.module.css";

const variants: SurfaceVariant[] = [
  "plain",
  "surface-1",
  "surface-2",
  "surface-3",
  "overlay",
  "selected",
];

function SurfaceGallery() {
  return (
    <main className={styles.gallery}>
      <section className={styles.section}>
        <h2 className={styles.title}>Surface variants</h2>
        <p className={styles.note}>
          Plain is panel-less: no fill, border, or shadow. Higher variants are
          reserved for true containment, overlays, and selected state.
        </p>
        <div className={styles.grid}>
          {variants.map((variant) => (
            <Surface className={styles.sample} key={variant} variant={variant}>
              <span>{variant}</span>
            </Surface>
          ))}
        </div>
      </section>
      <section className={styles.narrow}>
        <Surface className={styles.sample} variant="surface-2">
          Narrow container check
        </Surface>
      </section>
    </main>
  );
}

const meta = {
  title: "Design System/Surface",
  component: SurfaceGallery,
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta<typeof SurfaceGallery>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Gallery: Story = {};

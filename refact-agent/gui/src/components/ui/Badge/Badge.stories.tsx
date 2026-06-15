import type { Meta, StoryObj } from "@storybook/react";
import { Badge, type BadgeTone } from ".";
import styles from "./Badge.stories.module.css";

const tones: BadgeTone[] = [
  "default",
  "accent",
  "success",
  "warning",
  "danger",
  "muted",
];

function BadgeGallery() {
  return (
    <main className={styles.gallery}>
      <h2 className={styles.title}>Badge tones</h2>
      <div className={styles.row}>
        {tones.map((tone) => (
          <Badge key={tone} tone={tone}>
            {tone}
          </Badge>
        ))}
      </div>
      <section className={styles.narrow}>
        <Badge tone="accent">narrow label</Badge>
      </section>
    </main>
  );
}

const meta = {
  title: "Design System/Badge",
  component: BadgeGallery,
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta<typeof BadgeGallery>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Gallery: Story = {};

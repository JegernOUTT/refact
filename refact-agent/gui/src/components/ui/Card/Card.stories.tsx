import type { Meta, StoryObj } from "@storybook/react";
import { Card } from ".";
import styles from "./Card.stories.module.css";

function CardGallery() {
  return (
    <main className={styles.gallery}>
      <header className={styles.header}>
        <h2 className={styles.title}>Cards are restrained surfaces</h2>
        <p className={styles.note}>
          Panel-less content stays the default. Reach for Card only for
          overlays, selected state, or true containment.
        </p>
      </header>
      <div className={styles.grid}>
        <Card>
          <h3>Default card</h3>
          <p>Thin border, surface-1 fill, and card radius.</p>
        </Card>
        <Card selected>
          <h3>Selected card</h3>
          <p>Accent-soft selected treatment without a heavy panel.</p>
        </Card>
      </div>
    </main>
  );
}

const meta = {
  title: "Design System/Card",
  component: CardGallery,
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta<typeof CardGallery>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Gallery: Story = {};

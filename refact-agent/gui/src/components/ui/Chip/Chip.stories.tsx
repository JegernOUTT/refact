import type { Meta, StoryObj } from "@storybook/react";
import { BookOpen, File, Search } from "lucide-react";
import { Chip } from ".";
import { Icon } from "../Icon";
import styles from "./Chip.stories.module.css";

function ChipGallery() {
  return (
    <main className={styles.gallery}>
      <h2 className={styles.title}>Chip states</h2>
      <div className={styles.row}>
        <Chip icon={<Icon icon={File} size="sm" />}>file.tsx</Chip>
        <Chip icon={<Icon icon={Search} size="sm" />} selected>
          selected search
        </Chip>
        <Chip
          icon={<Icon icon={BookOpen} size="sm" />}
          removable
          onRemove={() => undefined}
        >
          removable
        </Chip>
        <Chip disabled removable>
          disabled
        </Chip>
        <Chip radius="chip">chip radius</Chip>
      </div>
      <section className={styles.narrow}>
        <Chip icon={<Icon icon={File} size="sm" />}>
          very-long-file-name-that-truncates.tsx
        </Chip>
      </section>
    </main>
  );
}

const meta = {
  title: "Design System/Chip",
  component: ChipGallery,
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta<typeof ChipGallery>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Gallery: Story = {};

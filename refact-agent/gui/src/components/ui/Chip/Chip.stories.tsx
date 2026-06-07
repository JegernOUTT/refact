import type { Meta, StoryObj } from "@storybook/react";
import { FileIcon, MagnifyingGlassIcon, ReaderIcon } from "@radix-ui/react-icons";
import { Chip } from ".";
import styles from "./Chip.stories.module.css";

function ChipGallery() {
  return (
    <main className={styles.gallery}>
      <h2 className={styles.title}>Chip states</h2>
      <div className={styles.row}>
        <Chip icon={<FileIcon />}>file.tsx</Chip>
        <Chip icon={<MagnifyingGlassIcon />} selected>
          selected search
        </Chip>
        <Chip icon={<ReaderIcon />} removable onRemove={() => undefined}>
          removable
        </Chip>
        <Chip disabled removable>
          disabled
        </Chip>
        <Chip radius="chip">chip radius</Chip>
      </div>
      <section className={styles.narrow}>
        <Chip icon={<FileIcon />}>very-long-file-name-that-truncates.tsx</Chip>
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

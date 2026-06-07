import type { Meta, StoryObj } from "@storybook/react";

import { VirtualList } from "./VirtualList";
import styles from "./VirtualList.stories.module.css";

interface MemoryRow {
  id: string;
  title: string;
  updated: string;
}

const items: MemoryRow[] = Array.from({ length: 1_000 }, (_, index) => ({
  id: `memory-${index + 1}`,
  title: `Knowledge memory ${index + 1}`,
  updated: `${(index % 28) + 1} days ago`,
}));

function VirtualListDemo() {
  return (
    <div className={styles.frame}>
      <VirtualList
        footer="End of virtualized results"
        getItemKey={(item) => item.id}
        header={`${items.length} memories`}
        items={items}
        renderItem={(item) => (
          <article className={styles.row}>
            <div className={styles.title}>{item.title}</div>
            <div className={styles.meta}>Updated {item.updated}</div>
          </article>
        )}
      />
    </div>
  );
}

const meta = {
  title: "UI/VirtualList",
  parameters: {
    layout: "centered",
  },
} satisfies Meta;

export default meta;

type Story = StoryObj<typeof meta>;

export const LargeList: Story = {
  render: () => <VirtualListDemo />,
};

export const LightDark: Story = {
  render: () => <VirtualListDemo />,
};

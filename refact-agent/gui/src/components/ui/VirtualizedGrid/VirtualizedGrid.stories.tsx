import type { Meta, StoryObj } from "@storybook/react";

import { VirtualizedGrid } from "./VirtualizedGrid";
import styles from "./VirtualizedGrid.stories.module.css";

interface DemoItem {
  id: string;
  title: string;
  subtitle: string;
}

const makeItems = (count: number): DemoItem[] =>
  Array.from({ length: count }, (_, index) => ({
    id: `item-${index + 1}`,
    title: `Tile ${index + 1}`,
    subtitle: `Uniform tile subtitle ${index + 1}`,
  }));

function GridDemo({ count, columns }: { count: number; columns?: number }) {
  return (
    <div className={styles.frame}>
      <VirtualizedGrid
        items={makeItems(count)}
        columns={columns}
        rowHeight={columns === 1 ? undefined : 120}
        getItemKey={(item) => item.id}
        renderItem={(item) => (
          <article className={styles.tile}>
            <span className={styles.title}>{item.title}</span>
            <span className={styles.meta}>{item.subtitle}</span>
          </article>
        )}
      />
    </div>
  );
}

const meta = {
  title: "UI/VirtualizedGrid",
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta;

export default meta;

type Story = StoryObj<typeof meta>;

export const ResponsiveGrid: Story = {
  render: () => <GridDemo count={24} />,
};

export const SingleColumn: Story = {
  render: () => <GridDemo count={24} columns={1} />,
};

export const Virtualized: Story = {
  render: () => <GridDemo count={500} />,
};

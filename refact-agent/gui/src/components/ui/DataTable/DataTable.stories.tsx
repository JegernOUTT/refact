import type { Meta, StoryObj } from "@storybook/react";

import { DataTable } from "./DataTable";
import type { DataTableColumn } from "./DataTable";
import styles from "./DataTable.stories.module.css";

interface ProviderRow {
  id: string;
  name: string;
  status: string;
  model: string;
  latency: number;
  notes: string;
}

const rows: ProviderRow[] = [
  {
    id: "openai",
    latency: 420,
    model: "gpt-4.1",
    name: "OpenAI",
    notes: "General chat and tools",
    status: "Ready",
  },
  {
    id: "anthropic",
    latency: 510,
    model: "claude-sonnet-4",
    name: "Anthropic",
    notes: "Reasoning and long context",
    status: "Ready",
  },
  {
    id: "local",
    latency: 82,
    model: "qwen-coder",
    name: "Local",
    notes: "Developer preview endpoint with intentionally long raw metadata",
    status: "Testing",
  },
];

const columns: DataTableColumn<ProviderRow>[] = [
  {
    cell: (row) => row.name,
    header: "Provider",
    id: "name",
    sortValue: (row) => row.name,
  },
  {
    cell: (row) => <span className={styles.status}>{row.status}</span>,
    header: "Status",
    id: "status",
    sortValue: (row) => row.status,
  },
  {
    cell: (row) => row.model,
    header: "Model",
    id: "model",
    sortValue: (row) => row.model,
  },
  {
    align: "end",
    cell: (row) => `${row.latency} ms`,
    header: "Latency",
    id: "latency",
    sortValue: (row) => row.latency,
  },
  {
    cell: (row) => row.notes,
    header: "Notes",
    id: "notes",
  },
];

const wideColumns: DataTableColumn<ProviderRow>[] = columns.map((column) => ({
  ...column,
  cell: (row) => <span className={styles.wideCell}>{column.cell(row)}</span>,
}));

function TableDemo({ narrow = false, wide = false }: { narrow?: boolean; wide?: boolean }) {
  return (
    <div className={narrow ? styles.narrow : styles.stack}>
      <DataTable
        caption={wide ? "Wide/raw mode keeps an intentional scroll island" : "Responsive providers"}
        columns={wide ? wideColumns : columns}
        enableSorting
        getRowId={(row) => row.id}
        rows={rows}
        wide={wide}
      />
    </div>
  );
}

const meta = {
  title: "UI/DataTable",
  parameters: {
    layout: "centered",
  },
} satisfies Meta;

export default meta;

type Story = StoryObj<typeof meta>;

export const Wide: Story = {
  render: () => <TableDemo wide />,
};

export const NarrowStacked: Story = {
  render: () => <TableDemo narrow />,
};

export const LightDark: Story = {
  render: () => <TableDemo />,
};

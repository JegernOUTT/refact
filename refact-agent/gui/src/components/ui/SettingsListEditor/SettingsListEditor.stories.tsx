import type { Meta, StoryObj } from "@storybook/react";
import { useState } from "react";

import { SettingsListEditor } from "./SettingsListEditor";
import type { SettingsListEditorItem } from "./SettingsListEditor";

let nextId = 0;

function createItem(value = ""): SettingsListEditorItem {
  nextId += 1;
  return { id: `item-${nextId}`, value };
}

function SettingsListEditorDemo({
  initialItems = [],
  monospace = false,
  placeholder,
  readOnly = false,
  addLabel,
  emptyLabel,
}: {
  initialItems?: SettingsListEditorItem[];
  monospace?: boolean;
  placeholder?: string;
  readOnly?: boolean;
  addLabel?: string;
  emptyLabel?: string;
}) {
  const [items, setItems] = useState<SettingsListEditorItem[]>(initialItems);

  return (
    <SettingsListEditor
      addLabel={addLabel}
      emptyLabel={emptyLabel}
      items={items}
      monospace={monospace}
      placeholder={placeholder}
      readOnly={readOnly}
      onAdd={() => setItems((current) => [...current, createItem()])}
      onChange={(id, value) =>
        setItems((current) =>
          current.map((item) => (item.id === id ? { ...item, value } : item)),
        )
      }
      onRemove={(id) =>
        setItems((current) => current.filter((item) => item.id !== id))
      }
    />
  );
}

const meta = {
  title: "UI/SettingsListEditor",
  parameters: {
    layout: "centered",
  },
} satisfies Meta;

export default meta;

type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: () => (
    <SettingsListEditorDemo
      addLabel="Add entry"
      initialItems={[
        createItem("first entry"),
        createItem("second entry"),
        createItem("third entry"),
      ]}
      placeholder="Entry value"
    />
  ),
};

export const Monospace: Story = {
  render: () => (
    <SettingsListEditorDemo
      addLabel="Add rule"
      initialItems={[
        createItem("sudo"),
        createItem("doas"),
        createItem("raw::(){ :|:& };:"),
      ]}
      monospace
      placeholder="Shell rule"
    />
  ),
};

export const ReadOnly: Story = {
  render: () => (
    <SettingsListEditorDemo
      initialItems={[createItem("sudo"), createItem("doas")]}
      monospace
      readOnly
    />
  ),
};

export const Empty: Story = {
  render: () => (
    <SettingsListEditorDemo
      addLabel="Add entry"
      emptyLabel="No entries yet."
      placeholder="Entry value"
    />
  ),
};

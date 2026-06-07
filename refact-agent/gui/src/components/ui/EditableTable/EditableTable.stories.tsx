import type { Meta, StoryObj } from "@storybook/react";
import { useState } from "react";

import { EditableTable } from "./EditableTable";
import type { EditableTableColumn } from "./EditableTable";
import styles from "./EditableTable.stories.module.css";

interface ParameterRow {
  name: string;
  description: string;
}

const columns: EditableTableColumn<ParameterRow>[] = [
  {
    header: "Name",
    id: "name",
    placeholder: "snake_case_name",
    width: "minmax(140px, 0.7fr)",
  },
  {
    header: "Description",
    id: "description",
    placeholder: "What this parameter controls",
  },
];

const createRow = (): ParameterRow => ({ description: "", name: "" });

function isSnakeCase(value: string) {
  return value.length === 0 || /^[a-z][a-z0-9_]*$/.test(value);
}

function EditableTableDemo() {
  const [rows, setRows] = useState<ParameterRow[]>([
    { description: "City for the forecast", name: "city" },
    { description: "Temperature unit", name: "Unit" },
  ]);

  return (
    <div className={styles.stack}>
      <p className={styles.note}>
        Press Enter inside a cell to move to the same column on the next row. Enter on the last row
        creates a new row and focuses it. The second row starts invalid to show cell validation.
      </p>
      <EditableTable
        addLabel="Add parameter"
        columns={columns}
        createRow={createRow}
        value={rows}
        validate={({ columnId, value }) => {
          if (columnId === "name" && !isSnakeCase(value)) {
            return "Use snake_case.";
          }

          return null;
        }}
        onChange={setRows}
      />
      <pre className={styles.preview}>{JSON.stringify(rows, null, 2)}</pre>
    </div>
  );
}

const meta = {
  title: "UI/EditableTable",
  parameters: {
    layout: "centered",
  },
} satisfies Meta;

export default meta;

type Story = StoryObj<typeof meta>;

export const AddRemoveEnterValidation: Story = {
  render: () => <EditableTableDemo />,
};

export const LightDark: Story = {
  render: () => <EditableTableDemo />,
};

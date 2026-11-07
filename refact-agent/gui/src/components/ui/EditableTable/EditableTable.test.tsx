import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import type { EditableTableColumn } from "./EditableTable";
import { EditableTable } from "./EditableTable";

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

function ControlledEditableTable({
  initialRows = [{ description: "City for the forecast", name: "city" }],
  onRowsChange,
}: {
  initialRows?: ParameterRow[];
  onRowsChange?: (rows: ParameterRow[]) => void;
}) {
  const [rows, setRows] = useState<ParameterRow[]>(initialRows);

  return (
    <EditableTable
      addLabel="Add parameter"
      columns={columns}
      createRow={createRow}
      emptyMessage="No parameters"
      removeLabel="Remove parameter"
      validate={({ columnId, value }) =>
        columnId === "name" && !isSnakeCase(value)
          ? "Use snake_case names"
          : null
      }
      value={rows}
      onChange={(nextRows) => {
        setRows(nextRows);
        onRowsChange?.(nextRows);
      }}
    />
  );
}

describe("EditableTable", () => {
  it("renders stacked labels associated with each editable cell", () => {
    render(
      <ControlledEditableTable
        initialRows={[
          { description: "City for the forecast", name: "city" },
          { description: "Temperature unit", name: "unit" },
        ]}
      />,
    );

    const nameInputs = screen.getAllByLabelText("Name");
    const descriptionInputs = screen.getAllByLabelText("Description");

    expect(nameInputs).toHaveLength(2);
    expect(descriptionInputs).toHaveLength(2);
    expect(nameInputs[0]).toHaveAttribute("id", "editable-table-0-name");
    expect(descriptionInputs[1]).toHaveAttribute(
      "id",
      "editable-table-1-description",
    );
  });

  it("preserves controlled add, remove, edit, and validation behavior", async () => {
    const user = userEvent.setup();
    const onRowsChange = vi.fn();

    render(
      <ControlledEditableTable
        initialRows={[{ description: "Temperature unit", name: "Unit" }]}
        onRowsChange={onRowsChange}
      />,
    );

    expect(screen.getByRole("alert")).toHaveTextContent("Use snake_case names");
    expect(screen.getByLabelText("Name")).toHaveAttribute(
      "aria-invalid",
      "true",
    );

    await user.clear(screen.getByLabelText("Name"));
    await user.type(screen.getByLabelText("Name"), "unit");

    expect(onRowsChange).toHaveBeenLastCalledWith([
      { description: "Temperature unit", name: "unit" },
    ]);
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Add parameter" }));

    expect(screen.getAllByLabelText("Name")).toHaveLength(2);
    expect(onRowsChange).toHaveBeenLastCalledWith([
      { description: "Temperature unit", name: "unit" },
      { description: "", name: "" },
    ]);

    await user.click(
      screen.getAllByRole("button", { name: "Remove parameter" })[0],
    );

    expect(screen.getAllByLabelText("Name")).toHaveLength(1);
    expect(onRowsChange).toHaveBeenLastCalledWith([
      { description: "", name: "" },
    ]);
  });

  it("keeps Enter-to-advance behavior for the active column", async () => {
    const user = userEvent.setup();

    render(<ControlledEditableTable />);

    await user.click(screen.getByLabelText("Name"));
    await user.keyboard("{Enter}");

    const nameInputs = screen.getAllByLabelText("Name");

    expect(nameInputs).toHaveLength(2);
    expect(nameInputs[1]).toHaveFocus();
  });
});

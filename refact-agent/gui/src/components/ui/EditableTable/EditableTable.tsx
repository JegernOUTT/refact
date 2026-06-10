import classNames from "classnames";
import { Plus, Trash2 } from "lucide-react";
import React, { useEffect, useMemo, useRef, useState } from "react";

import { Button, IconButton } from "../Button";
import { FieldError, FieldText } from "../Field";
import styles from "./EditableTable.module.css";

export type EditableTableRow = object;

export interface EditableTableColumn<T extends EditableTableRow> {
  id: Extract<keyof T, string>;
  header: React.ReactNode;
  placeholder?: string;
  inputType?: React.ComponentProps<"input">["type"];
  width?: string;
  getInputProps?: (params: {
    row: T;
    rowIndex: number;
  }) => Record<string, unknown>;
}

export type EditableTableValidate<T extends EditableTableRow> = (params: {
  row: T;
  rowIndex: number;
  columnId: Extract<keyof T, string>;
  value: string;
}) => React.ReactNode;

export interface EditableTableProps<T extends EditableTableRow>
  extends Omit<React.ComponentProps<"div">, "children" | "onChange"> {
  columns: EditableTableColumn<T>[];
  value: T[];
  onChange: (value: T[]) => void;
  createRow: () => T;
  validate?: EditableTableValidate<T>;
  addLabel?: string;
  removeLabel?: string;
  emptyMessage?: React.ReactNode;
}

interface InternalRow<T extends EditableTableRow> {
  id: string;
  value: T;
}

let editableTableId = 0;

const nextId = () => `editable-row-${++editableTableId}`;

export function EditableTable<T extends EditableTableRow>({
  addLabel = "Add row",
  className,
  columns,
  createRow,
  emptyMessage = "No rows yet",
  onChange,
  removeLabel = "Remove row",
  validate,
  value,
  ...props
}: EditableTableProps<T>) {
  const [rows, setRows] = useState<InternalRow<T>[]>(() =>
    value.map((row) => ({ id: nextId(), value: row })),
  );
  const pendingFocusRef = useRef<{ rowIndex: number; columnId: string } | null>(
    null,
  );
  const inputRefs = useRef(new Map<string, HTMLInputElement>());

  useEffect(() => {
    setRows((currentRows) =>
      value.map((row, index) => ({
        id: currentRows[index]?.id ?? nextId(),
        value: row,
      })),
    );
  }, [value]);

  useEffect(() => {
    const pendingFocus = pendingFocusRef.current;

    if (!pendingFocus) {
      return;
    }

    pendingFocusRef.current = null;
    inputRefs.current
      .get(inputKey(pendingFocus.rowIndex, pendingFocus.columnId))
      ?.focus();
  }, [rows]);

  const errors = useMemo(
    () =>
      rows.map(
        (row, rowIndex) =>
          Object.fromEntries(
            columns.map((column) => [
              column.id,
              validate?.({
                columnId: column.id,
                row: row.value,
                rowIndex,
                value: String(row.value[column.id]),
              }) ?? null,
            ]),
          ) as Partial<Record<Extract<keyof T, string>, React.ReactNode>>,
      ),
    [columns, rows, validate],
  );

  const emitChange = (nextRows: InternalRow<T>[]) => {
    setRows(nextRows);
    onChange(nextRows.map((row) => row.value));
  };

  const updateCell = (
    rowIndex: number,
    columnId: Extract<keyof T, string>,
    nextValue: string,
  ) => {
    emitChange(
      rows.map((row, index) =>
        index === rowIndex
          ? { ...row, value: { ...row.value, [columnId]: nextValue } }
          : row,
      ),
    );
  };

  const addRow = () => {
    emitChange([...rows, { id: nextId(), value: createRow() }]);
  };

  const removeRow = (rowIndex: number) => {
    emitChange(rows.filter((_, index) => index !== rowIndex));
  };

  const focusNext = (rowIndex: number, columnId: Extract<keyof T, string>) => {
    const nextRowIndex = rowIndex + 1;

    if (nextRowIndex < rows.length) {
      inputRefs.current.get(inputKey(nextRowIndex, columnId))?.focus();
      return;
    }

    pendingFocusRef.current = { rowIndex: nextRowIndex, columnId };
    emitChange([...rows, { id: nextId(), value: createRow() }]);
  };

  const tableStyle = {
    "--editable-table-columns": `${columns
      .map((column) => column.width ?? "minmax(0, 1fr)")
      .join(" ")} auto`,
  } as React.CSSProperties;

  return (
    <div {...props} className={classNames(styles.root, className)}>
      <div className={styles.tableWrap}>
        <table className={styles.table} style={tableStyle}>
          <thead>
            <tr className={styles.row}>
              {columns.map((column) => (
                <th className={styles.headerCell} key={column.id} scope="col">
                  {column.header}
                </th>
              ))}
              <th className={styles.headerCell} scope="col">
                <span className={styles.srOnly}>Actions</span>
              </th>
            </tr>
          </thead>
          <tbody className="rf-stagger">
            {rows.length ? (
              rows.map((row, rowIndex) => (
                <tr className={classNames(styles.row, "rf-enter")} key={row.id}>
                  {columns.map((column) => {
                    const error = errors[rowIndex]?.[column.id];

                    const inputProps = (column.getInputProps?.({
                      row: row.value,
                      rowIndex,
                    }) ?? {}) as Partial<React.ComponentProps<"input">>;

                    return (
                      <td className={styles.cell} key={column.id}>
                        <label
                          className={styles.stackedLabel}
                          htmlFor={inputKey(rowIndex, column.id)}
                        >
                          {column.header}
                        </label>
                        <FieldText
                          {...inputProps}
                          aria-invalid={error ? true : undefined}
                          id={inputKey(rowIndex, column.id)}
                          ref={(node) => {
                            const key = inputKey(rowIndex, column.id);

                            if (node) {
                              inputRefs.current.set(key, node);
                            } else {
                              inputRefs.current.delete(key);
                            }
                          }}
                          placeholder={column.placeholder}
                          type={column.inputType}
                          value={String(row.value[column.id])}
                          onChange={(nextValue) =>
                            updateCell(rowIndex, column.id, nextValue)
                          }
                          onKeyDown={(event) => {
                            if (event.key === "Enter") {
                              event.preventDefault();
                              focusNext(rowIndex, column.id);
                            }
                          }}
                        />
                        {error ? (
                          <FieldError className={styles.error}>
                            {error}
                          </FieldError>
                        ) : null}
                      </td>
                    );
                  })}
                  <td className={styles.actionCell}>
                    <IconButton
                      aria-label={removeLabel}
                      icon={Trash2}
                      size="sm"
                      type="button"
                      variant="danger"
                      onClick={() => removeRow(rowIndex)}
                    />
                  </td>
                </tr>
              ))
            ) : (
              <tr className={styles.row}>
                <td className={styles.emptyCell} colSpan={columns.length + 1}>
                  {emptyMessage}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      <Button
        leftIcon={Plus}
        size="sm"
        type="button"
        variant="soft"
        onClick={addRow}
      >
        {addLabel}
      </Button>
    </div>
  );
}

function inputKey(rowIndex: number, columnId: string) {
  return `editable-table-${rowIndex}-${columnId}`;
}

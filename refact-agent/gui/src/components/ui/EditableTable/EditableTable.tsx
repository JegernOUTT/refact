import classNames from "classnames";
import { Eye, EyeOff, Plus, Trash2 } from "lucide-react";
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
  secret?: boolean | ((row: T) => boolean);
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
  getRowId?: (row: T) => string;
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
  getRowId,
  onChange,
  removeLabel = "Remove row",
  validate,
  value,
  ...props
}: EditableTableProps<T>) {
  const [rows, setRows] = useState<InternalRow<T>[]>(() =>
    value.map((row) => ({ id: getRowId?.(row) ?? nextId(), value: row })),
  );
  const pendingFocusRef = useRef<{ rowIndex: number; columnId: string } | null>(
    null,
  );
  const inputRefs = useRef(new Map<string, HTMLInputElement>());

  useEffect(() => {
    setRows((currentRows) =>
      value.map((row, index) => {
        const currentRow = currentRows[index] as InternalRow<T> | undefined;

        return {
          id: getRowId?.(row) ?? currentRow?.id ?? nextId(),
          value: row,
        };
      }),
    );
  }, [getRowId, value]);

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

  const createInternalRow = (): InternalRow<T> => {
    const nextValue = createRow();

    return { id: getRowId?.(nextValue) ?? nextId(), value: nextValue };
  };

  const addRow = () => {
    emitChange([...rows, createInternalRow()]);
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
    emitChange([...rows, createInternalRow()]);
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
                    const secret =
                      typeof column.secret === "function"
                        ? column.secret(row.value)
                        : column.secret ?? false;

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
                        <EditableTableInput
                          error={error}
                          id={inputKey(rowIndex, column.id)}
                          inputProps={inputProps}
                          inputType={column.inputType}
                          key={`${inputKey(rowIndex, column.id)}-${
                            secret ? "secret" : "plain"
                          }`}
                          placeholder={column.placeholder}
                          secret={secret}
                          value={String(row.value[column.id])}
                          inputRef={(node) => {
                            const key = inputKey(rowIndex, column.id);

                            if (node) {
                              inputRefs.current.set(key, node);
                            } else {
                              inputRefs.current.delete(key);
                            }
                          }}
                          onChange={(nextValue) =>
                            updateCell(rowIndex, column.id, nextValue)
                          }
                          onEnter={() => focusNext(rowIndex, column.id)}
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

interface EditableTableInputProps {
  error: React.ReactNode;
  id: string;
  inputProps: Partial<React.ComponentProps<"input">>;
  inputRef: (node: HTMLInputElement | null) => void;
  inputType?: React.ComponentProps<"input">["type"];
  placeholder?: string;
  secret: boolean;
  value: string;
  onChange: (value: string) => void;
  onEnter: () => void;
}

function EditableTableInput({
  error,
  id,
  inputProps,
  inputRef,
  inputType,
  onChange,
  onEnter,
  placeholder,
  secret,
  value,
}: EditableTableInputProps) {
  const [revealed, setRevealed] = useState(false);

  const input = (
    <FieldText
      {...inputProps}
      aria-invalid={error ? true : undefined}
      autoComplete={secret ? "off" : inputProps.autoComplete}
      data-1p-ignore={secret ? "" : undefined}
      id={id}
      placeholder={placeholder}
      ref={inputRef}
      spellCheck={secret ? false : inputProps.spellCheck}
      type={secret ? (revealed ? "text" : "password") : inputType}
      value={value}
      onChange={onChange}
      onKeyDown={(event) => {
        inputProps.onKeyDown?.(event);

        if (!event.defaultPrevented && event.key === "Enter") {
          event.preventDefault();
          onEnter();
        }
      }}
    />
  );

  if (!secret) {
    return input;
  }

  return (
    <div className={styles.secretField}>
      {input}
      <IconButton
        aria-label={revealed ? "Hide value" : "Show value"}
        icon={revealed ? EyeOff : Eye}
        size="sm"
        type="button"
        variant="ghost"
        onClick={() => setRevealed((current) => !current)}
      />
    </div>
  );
}

function inputKey(rowIndex: number, columnId: string) {
  return `editable-table-${rowIndex}-${columnId}`;
}

import { FC, useCallback, useEffect, useMemo, useState } from "react";
import isEqual from "lodash.isequal";

import { EditableTable } from "../../ui";
import { debugIntegrations } from "../../../debugConfig";
import { MCPEnvs } from "../../../services/refact";

import styles from "./IntegrationTables.module.css";

type KeyValueTableProps = {
  initialData: Record<string, string>;
  onChange: (data: Record<string, string>) => void;
  columnNames?: string[];
  emptyMessage?: string;
};

type KeyValueRow = {
  id: string;
  key: string;
  value: string;
  originalKey: string;
  order: number;
};

let keyValueRowId = 0;

export const KeyValueTable: FC<KeyValueTableProps> = ({
  initialData,
  onChange,
  columnNames = ["Key", "Value"],
  emptyMessage,
}) => {
  const [nextOrder, setNextOrder] = useState(
    () => Object.keys(initialData).length,
  );
  const [rows, setRows] = useState<KeyValueRow[]>(() => makeRows(initialData));
  const [previousData, setPreviousData] = useState<MCPEnvs>(() => initialData);
  const [previousInitialData, setPreviousInitialData] =
    useState<Record<string, string>>(initialData);

  const tableData = useMemo(
    () => [...rows].sort((a, b) => a.order - b.order),
    [rows],
  );

  const duplicateKeys = useMemo(
    () => findDuplicateKeys(tableData),
    [tableData],
  );

  const emittedData = useMemo(
    () => (duplicateKeys.size === 0 ? rowsToData(tableData) : null),
    [duplicateKeys, tableData],
  );

  const updateData = useCallback(() => {
    if (!emittedData || isEqual(previousData, emittedData)) {
      return;
    }

    setPreviousData(emittedData);
    onChange(emittedData);
  }, [emittedData, onChange, previousData]);

  useEffect(() => {
    if (!isEqual(previousInitialData, initialData)) {
      setPreviousInitialData(initialData);
      setRows(makeRows(initialData));
      setPreviousData(initialData);
      setNextOrder(Object.keys(initialData).length);
    }
  }, [initialData, previousInitialData]);

  useEffect(() => {
    updateData();
  }, [updateData]);

  useEffect(() => {
    debugIntegrations(`[DEBUG]: KeyValueTable data changed: `, tableData);
  }, [tableData]);

  const handleRowsChange = useCallback((nextRows: KeyValueRow[]) => {
    setRows(nextRows);
  }, []);

  const getRowId = useCallback((row: KeyValueRow) => row.id, []);

  const createRow = useCallback((): KeyValueRow => {
    const existingKeys = new Set(rows.map((row) => row.key));
    const nextKeyOrder = findNextKeyOrder(nextOrder, existingKeys);

    setNextOrder(nextKeyOrder + 1);

    return {
      id: nextRowId(),
      key: String(nextKeyOrder),
      value: "",
      originalKey: "",
      order: nextOrder,
    };
  }, [nextOrder, rows]);

  return (
    <EditableTable<KeyValueRow>
      addLabel="Add row"
      className={styles.table}
      columns={[
        {
          id: "key",
          header: columnNames[0],
          getInputProps: ({ row, rowIndex }) => ({
            "data-row-id": row.id,
            "data-row-index": rowIndex,
            "data-field": "key",
            "data-next-row": row.originalKey,
          }),
        },
        {
          id: "value",
          header: columnNames[1],
          getInputProps: ({ row, rowIndex }) => ({
            "data-row-id": row.id,
            "data-row-index": rowIndex,
            "data-field": "value",
            "data-next-row": row.originalKey,
          }),
        },
      ]}
      createRow={createRow}
      emptyMessage={emptyMessage}
      removeLabel="Remove"
      getRowId={getRowId}
      validate={({ columnId, value }) => {
        if (columnId !== "key" || !duplicateKeys.has(value)) {
          return null;
        }

        return `Duplicate key "${value}" is already used.`;
      }}
      value={tableData}
      onChange={handleRowsChange}
    />
  );
};

function makeRows(data: Record<string, string>): KeyValueRow[] {
  return Object.entries(data).map(([key, value], order) => ({
    id: nextRowId(),
    key,
    value,
    originalKey: key,
    order,
  }));
}

function rowsToData(rows: KeyValueRow[]): MCPEnvs {
  return Object.fromEntries(rows.map((row) => [row.key, row.value]));
}

function findDuplicateKeys(rows: KeyValueRow[]): Set<string> {
  const counts = new Map<string, number>();

  rows.forEach((row) => {
    counts.set(row.key, (counts.get(row.key) ?? 0) + 1);
  });

  return new Set(
    Array.from(counts.entries())
      .filter(([, count]) => count > 1)
      .map(([key]) => key),
  );
}

function findNextKeyOrder(start: number, existingKeys: Set<string>): number {
  let candidate = start;

  while (existingKeys.has(String(candidate))) {
    candidate += 1;
  }

  return candidate;
}

function nextRowId(): string {
  keyValueRowId += 1;
  return `key-value-row-${keyValueRowId}`;
}

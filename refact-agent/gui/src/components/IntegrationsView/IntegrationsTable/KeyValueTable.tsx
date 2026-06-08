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
  key: string;
  value: string;
  originalKey: string;
  order: number;
};

export const KeyValueTable: FC<KeyValueTableProps> = ({
  initialData,
  onChange,
  columnNames = ["Key", "Value"],
  emptyMessage,
}) => {
  const [nextOrder, setNextOrder] = useState(
    () => Object.keys(initialData).length,
  );
  const [data, setData] = useState<MCPEnvs>(() => initialData);
  const [previousData, setPreviousData] = useState<MCPEnvs>(() => initialData);
  const [previousInitialData, setPreviousInitialData] =
    useState<Record<string, string>>(initialData);
  const [keyOrders, setKeyOrders] = useState<Record<string, number>>(() =>
    makeKeyOrders(initialData),
  );

  const tableData = useMemo(
    () =>
      Object.entries(data)
        .map(
          ([key, value]): KeyValueRow => ({
            key,
            value,
            originalKey: key,
            order: keyOrders[key],
          }),
        )
        .sort((a, b) => a.order - b.order),
    [data, keyOrders],
  );

  const isDataChanged = useMemo(
    () => !isEqual(previousData, data),
    [previousData, data],
  );

  const updateData = useCallback(() => {
    setPreviousData(data);
    onChange(data);
  }, [data, onChange]);

  useEffect(() => {
    if (!isEqual(previousInitialData, initialData)) {
      setPreviousInitialData(initialData);
      setData(initialData);
      setPreviousData(initialData);
      setKeyOrders(makeKeyOrders(initialData));
      setNextOrder(Object.keys(initialData).length);
    }
  }, [initialData, previousInitialData]);

  useEffect(() => {
    if (isDataChanged) {
      updateData();
    }
  }, [updateData, isDataChanged]);

  useEffect(() => {
    debugIntegrations(`[DEBUG]: KeyValueTable data changed: `, tableData);
  }, [tableData]);

  const handleRowsChange = useCallback((nextRows: KeyValueRow[]) => {
    const nextData: MCPEnvs = {};
    const nextOrders: Record<string, number> = {};

    nextRows.forEach((row) => {
      nextData[row.key] = row.value;
      nextOrders[row.key] = row.order;
    });

    setData(nextData);
    setKeyOrders(nextOrders);
  }, []);

  const createRow = useCallback((): KeyValueRow => {
    const key = `${Object.keys(data).length}`;
    setNextOrder((order) => order + 1);
    return { key, value: "", originalKey: "", order: nextOrder };
  }, [data, nextOrder]);

  return (
    <EditableTable<KeyValueRow>
      addLabel="Add row"
      className={styles.table}
      columns={[
        {
          id: "key",
          header: columnNames[0],
          getInputProps: ({ row, rowIndex }) => ({
            "data-row-index": rowIndex,
            "data-field": "key",
            "data-next-row": row.originalKey,
          }),
        },
        {
          id: "value",
          header: columnNames[1],
          getInputProps: ({ row, rowIndex }) => ({
            "data-row-index": rowIndex,
            "data-field": "value",
            "data-next-row": row.originalKey,
          }),
        },
      ]}
      createRow={createRow}
      emptyMessage={emptyMessage}
      removeLabel="Remove"
      value={tableData}
      onChange={handleRowsChange}
    />
  );
};

function makeKeyOrders(data: Record<string, string>): Record<string, number> {
  return Object.fromEntries(
    Object.keys(data).map((key, index) => [key, index]),
  );
}

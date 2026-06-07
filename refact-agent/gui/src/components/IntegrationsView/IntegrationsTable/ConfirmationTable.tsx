import { FC, useCallback, useEffect, useMemo, useState } from "react";
import isEqual from "lodash.isequal";

import { EditableTable } from "../../ui";
import { toPascalCase } from "../../../utils/toPascalCase";

import styles from "./IntegrationTables.module.css";

type ConfirmationTableProps = {
  tableName: string;
  initialData: string[];
  onToolConfirmation: (key: string, data: string[]) => void;
};

type ConfirmationRow = {
  value: string;
};

export const ConfirmationTable: FC<ConfirmationTableProps> = ({
  tableName,
  initialData,
  onToolConfirmation,
}) => {
  const [data, setData] = useState<string[]>(initialData);
  const [previousData, setPreviousData] = useState<string[]>(initialData);

  const isDataChanged = useMemo(() => !isEqual(previousData, data), [previousData, data]);

  const updateData = useCallback(() => {
    setPreviousData(data);
    onToolConfirmation(tableName, data);
  }, [data, onToolConfirmation, tableName]);

  useEffect(() => {
    if (isDataChanged) {
      updateData();
    }
  }, [updateData, isDataChanged]);

  const rows = useMemo<ConfirmationRow[]>(() => data.map((value) => ({ value })), [data]);

  return (
    <EditableTable<ConfirmationRow>
      addLabel="Add row"
      className={styles.table}
      columns={[
        {
          id: "value",
          header: toPascalCase(tableName),
          getInputProps: ({ rowIndex }) => ({
            "data-row-index": rowIndex,
            "data-field": tableName,
            "data-next-row": rowIndex.toString(),
          }),
        },
      ]}
      createRow={() => ({ value: "" })}
      emptyMessage="No rules set yet"
      removeLabel="Remove"
      value={rows}
      onChange={(nextRows) => setData(nextRows.map((row) => row.value))}
    />
  );
};

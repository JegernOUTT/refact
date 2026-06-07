import { FC, useCallback, useEffect, useMemo, useState } from "react";
import isEqual from "lodash.isequal";

import { EditableTable } from "../../ui";
import { ToolParameterEntity } from "../../../services/refact";
import { validateSnakeCase } from "../../../utils/validateSnakeCase";
import { debugTables } from "../../../debugConfig";

import styles from "./IntegrationTables.module.css";

type ParametersTableProps = {
  initialData: ToolParameterEntity[];
  onToolParameters: (data: ToolParameterEntity[]) => void;
};

export const ParametersTable: FC<ParametersTableProps> = ({ initialData, onToolParameters }) => {
  const [data, setData] = useState<ToolParameterEntity[]>(initialData);
  const [previousData, setPreviousData] = useState<ToolParameterEntity[]>(initialData);

  const createRow = useCallback((): ToolParameterEntity => ({ name: "", description: "", type: "string" }), []);

  const isDataChanged = useMemo(() => !isEqual(previousData, data), [previousData, data]);

  const updateData = useCallback(() => {
    setPreviousData(data);
    onToolParameters(data);
  }, [data, onToolParameters]);

  useEffect(() => {
    if (isDataChanged) {
      updateData();
    }
  }, [updateData, isDataChanged]);

  return (
    <EditableTable<ToolParameterEntity>
      addLabel="Add row"
      className={styles.table}
      columns={[
        {
          id: "name",
          header: "Name",
          getInputProps: ({ rowIndex }) => ({
            "data-row-index": rowIndex,
            "data-field": "name",
            "data-next-row": rowIndex.toString(),
          }),
        },
        {
          id: "description",
          header: "Description",
          getInputProps: ({ rowIndex }) => ({
            "data-row-index": rowIndex,
            "data-field": "description",
            "data-next-row": rowIndex.toString(),
          }),
        },
      ]}
      createRow={createRow}
      emptyMessage="No parameters set yet"
      removeLabel="Remove"
      validate={({ columnId, value }) => {
        if (columnId !== "name" || validateSnakeCase(value)) {
          return null;
        }

        debugTables(`[DEBUG VALIDATION]: field ${columnId} is not written in snake case`);
        return `The value "${value}" must be written in snake case.`;
      }}
      value={data}
      onChange={setData}
    />
  );
};

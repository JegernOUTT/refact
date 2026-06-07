import React, { useCallback } from "react";

import { EditableTable, Field } from "../../../components/ui";

export type ToolConfirmRule = {
  match: string;
  action: string;
};

type RulesTableEditorProps = {
  value: ToolConfirmRule[];
  onChange: (value: ToolConfirmRule[]) => void;
  label?: string;
};

export const RulesTableEditor: React.FC<RulesTableEditorProps> = ({
  value,
  onChange,
  label = "Tool Confirmation Rules",
}) => {
  const createRow = useCallback(
    (): ToolConfirmRule => ({ match: "*", action: "ask" }),
    [],
  );

  return (
    <Field label={label}>
      <EditableTable<ToolConfirmRule>
        addLabel="Add Rule"
        columns={[
          {
            id: "match",
            header: "Pattern",
            placeholder: "Pattern (e.g., shell:*)",
          },
          {
            id: "action",
            header: "Action",
            placeholder: "auto / allow / deny / ask",
            width: "minmax(8rem, 0.45fr)",
          },
        ]}
        createRow={createRow}
        emptyMessage="No rules defined"
        value={value}
        onChange={onChange}
      />
    </Field>
  );
};

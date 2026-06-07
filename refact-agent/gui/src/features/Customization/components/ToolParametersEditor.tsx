import React, { useCallback } from "react";
import { Plus, Trash2 } from "lucide-react";

import {
  Button,
  Field,
  FieldSelect,
  FieldSwitch,
  FieldText,
  IconButton,
} from "../../../components/ui";
import styles from "./editors.module.css";

export type ToolParameter = {
  name: string;
  type: string;
  description: string;
  default?: unknown;
};

type ToolParametersEditorProps = {
  parameters: ToolParameter[];
  required: string[];
  onParametersChange: (value: ToolParameter[]) => void;
  onRequiredChange: (value: string[]) => void;
  label?: string;
};

const PARAM_TYPES = [
  "string",
  "integer",
  "number",
  "boolean",
  "array",
  "object",
];

const PARAM_TYPE_OPTIONS = PARAM_TYPES.map((type) => ({ value: type, label: type }));

export const ToolParametersEditor: React.FC<ToolParametersEditorProps> = ({
  parameters,
  required,
  onParametersChange,
  onRequiredChange,
  label = "Tool Parameters",
}) => {
  const addParameter = useCallback(() => {
    onParametersChange([
      ...parameters,
      { name: "", type: "string", description: "" },
    ]);
  }, [parameters, onParametersChange]);

  const removeParameter = useCallback(
    (index: number) => {
      const param = parameters[index] as ToolParameter | undefined;
      onParametersChange(parameters.filter((_, i) => i !== index));
      if (param !== undefined && required.includes(param.name)) {
        onRequiredChange(required.filter((r) => r !== param.name));
      }
    },
    [parameters, required, onParametersChange, onRequiredChange],
  );

  const updateParameter = useCallback(
    (index: number, field: keyof ToolParameter, value: string) => {
      const oldName = parameters[index].name;
      const newParams = parameters.map((p, i) =>
        i === index ? { ...p, [field]: value } : p,
      );
      onParametersChange(newParams);
      if (field === "name" && required.includes(oldName)) {
        onRequiredChange(required.map((r) => (r === oldName ? value : r)));
      }
    },
    [parameters, required, onParametersChange, onRequiredChange],
  );

  const toggleRequired = useCallback(
    (name: string, isRequired: boolean) => {
      if (isRequired) {
        onRequiredChange([...required, name]);
      } else {
        onRequiredChange(required.filter((r) => r !== name));
      }
    },
    [required, onRequiredChange],
  );

  return (
    <Field label={label}>
      <div className={styles.tableEditorStack}>
        {parameters.length === 0 ? (
          <p className={styles.emptyText}>No parameters defined</p>
        ) : (
          <div className={styles.parameterGrid}>
            {parameters.map((param, index) => (
              <div className={styles.parameterRow} key={index}>
                <Field label="Name">
                  <FieldText
                    value={param.name}
                    onChange={(value) => updateParameter(index, "name", value)}
                    placeholder="param_name"
                  />
                </Field>
                <Field label="Type">
                  <FieldSelect
                    options={PARAM_TYPE_OPTIONS}
                    value={param.type}
                    onChange={(value) => updateParameter(index, "type", value)}
                  />
                </Field>
                <Field label="Description">
                  <FieldText
                    value={param.description}
                    onChange={(value) =>
                      updateParameter(index, "description", value)
                    }
                    placeholder="Description"
                  />
                </Field>
                <Field label="Required">
                  <FieldSwitch
                    checked={required.includes(param.name)}
                    disabled={!param.name}
                    onChange={(checked) => toggleRequired(param.name, checked)}
                  />
                </Field>
                <IconButton
                  aria-label={`Remove parameter ${index + 1}`}
                  icon={Trash2}
                  size="sm"
                  variant="danger"
                  onClick={() => removeParameter(index)}
                />
              </div>
            ))}
          </div>
        )}
        <Button leftIcon={Plus} size="sm" variant="soft" onClick={addParameter}>
          Add Parameter
        </Button>
      </div>
    </Field>
  );
};

import React, { useCallback } from "react";
import { Flex, Button, TextField, Select, IconButton, Text, Table, Checkbox } from "@radix-ui/themes";
import { PlusIcon, TrashIcon } from "@radix-ui/react-icons";

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

const PARAM_TYPES = ["string", "integer", "number", "boolean", "array", "object"];

export const ToolParametersEditor: React.FC<ToolParametersEditorProps> = ({
  parameters,
  required,
  onParametersChange,
  onRequiredChange,
  label = "Tool Parameters",
}) => {
  const addParameter = useCallback(() => {
    onParametersChange([...parameters, { name: "", type: "string", description: "" }]);
  }, [parameters, onParametersChange]);

  const removeParameter = useCallback((index: number) => {
    const param = parameters[index] as ToolParameter | undefined;
    onParametersChange(parameters.filter((_, i) => i !== index));
    if (param !== undefined && required.includes(param.name)) {
      onRequiredChange(required.filter((r) => r !== param.name));
    }
  }, [parameters, required, onParametersChange, onRequiredChange]);

  const updateParameter = useCallback((index: number, field: keyof ToolParameter, value: string) => {
    const oldName = parameters[index].name;
    const newParams = parameters.map((p, i) => i === index ? { ...p, [field]: value } : p);
    onParametersChange(newParams);
    if (field === "name" && required.includes(oldName)) {
      onRequiredChange(required.map((r) => r === oldName ? value : r));
    }
  }, [parameters, required, onParametersChange, onRequiredChange]);

  const toggleRequired = useCallback((name: string, isRequired: boolean) => {
    if (isRequired) {
      onRequiredChange([...required, name]);
    } else {
      onRequiredChange(required.filter((r) => r !== name));
    }
  }, [required, onRequiredChange]);

  return (
    <Flex direction="column" gap="2">
      <Flex justify="between" align="center">
        <Text size="2" weight="medium">{label}</Text>
        <Button size="1" variant="soft" onClick={addParameter}>
          <PlusIcon /> Add Parameter
        </Button>
      </Flex>
      {parameters.length === 0 ? (
        <Text size="1" color="gray">No parameters defined</Text>
      ) : (
        <Table.Root size="1">
          <Table.Header>
            <Table.Row>
              <Table.ColumnHeaderCell>Name</Table.ColumnHeaderCell>
              <Table.ColumnHeaderCell>Type</Table.ColumnHeaderCell>
              <Table.ColumnHeaderCell>Description</Table.ColumnHeaderCell>
              <Table.ColumnHeaderCell>Required</Table.ColumnHeaderCell>
              <Table.ColumnHeaderCell width="60px"></Table.ColumnHeaderCell>
            </Table.Row>
          </Table.Header>
          <Table.Body>
            {parameters.map((param, index) => (
              <Table.Row key={index}>
                <Table.Cell>
                  <TextField.Root
                    size="1"
                    value={param.name}
                    onChange={(e) => updateParameter(index, "name", e.target.value)}
                    placeholder="param_name"
                  />
                </Table.Cell>
                <Table.Cell>
                  <Select.Root value={param.type} onValueChange={(v) => updateParameter(index, "type", v)}>
                    <Select.Trigger />
                    <Select.Content>
                      {PARAM_TYPES.map((t) => (
                        <Select.Item key={t} value={t}>{t}</Select.Item>
                      ))}
                    </Select.Content>
                  </Select.Root>
                </Table.Cell>
                <Table.Cell>
                  <TextField.Root
                    size="1"
                    value={param.description}
                    onChange={(e) => updateParameter(index, "description", e.target.value)}
                    placeholder="Description"
                  />
                </Table.Cell>
                <Table.Cell>
                  <Checkbox
                    checked={required.includes(param.name)}
                    disabled={!param.name}
                    onCheckedChange={(checked) => toggleRequired(param.name, checked === true)}
                  />
                </Table.Cell>
                <Table.Cell>
                  <IconButton size="1" variant="ghost" color="red" onClick={() => removeParameter(index)}>
                    <TrashIcon />
                  </IconButton>
                </Table.Cell>
              </Table.Row>
            ))}
          </Table.Body>
        </Table.Root>
      )}
    </Flex>
  );
};

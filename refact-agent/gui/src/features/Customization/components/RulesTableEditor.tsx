import React, { useCallback, useState, useEffect } from "react";
import {
  Flex,
  Button,
  TextField,
  IconButton,
  Text,
  DropdownMenu,
} from "@radix-ui/themes";
import { PlusIcon, TrashIcon, ChevronDownIcon } from "@radix-ui/react-icons";

export type ToolConfirmRule = {
  match: string;
  action: string;
};

type InternalRule = { match: string; action: string; _id: string };

type RulesTableEditorProps = {
  value: ToolConfirmRule[];
  onChange: (value: ToolConfirmRule[]) => void;
  label?: string;
};

const COMMON_ACTIONS = ["auto", "allow", "deny", "ask"];

let idCounter = 0;
const generateId = () => `rule_${++idCounter}_${Date.now()}`;

const toInternal = (rules: ToolConfirmRule[]): InternalRule[] =>
  rules.map((r) => ({ ...r, _id: generateId() }));

const toExternal = (rules: InternalRule[]): ToolConfirmRule[] =>
  rules.map(({ _id, ...rest }) => rest);

export const RulesTableEditor: React.FC<RulesTableEditorProps> = ({
  value,
  onChange,
  label = "Tool Confirmation Rules",
}) => {
  const [internal, setInternal] = useState<InternalRule[]>(() =>
    toInternal(value),
  );
  const valueKey = JSON.stringify(value);

  useEffect(() => {
    setInternal(toInternal(value));
    // eslint-disable-next-line react-hooks/exhaustive-deps -- valueKey is derived from value, used for deep comparison
  }, [valueKey]);

  const emit = useCallback(
    (rules: InternalRule[]) => {
      setInternal(rules);
      onChange(toExternal(rules));
    },
    [onChange],
  );

  const addRule = useCallback(() => {
    emit([...internal, { match: "*", action: "ask", _id: generateId() }]);
  }, [internal, emit]);

  const removeRule = useCallback(
    (id: string) => {
      emit(internal.filter((r) => r._id !== id));
    },
    [internal, emit],
  );

  const updateRule = useCallback(
    (id: string, field: "match" | "action", fieldValue: string) => {
      emit(
        internal.map((r) => (r._id === id ? { ...r, [field]: fieldValue } : r)),
      );
    },
    [internal, emit],
  );

  return (
    <Flex direction="column" gap="2">
      <Flex justify="between" align="center">
        <Text size="2" weight="medium">
          {label}
        </Text>
        <Button size="1" variant="soft" onClick={addRule}>
          <PlusIcon /> Add Rule
        </Button>
      </Flex>
      {internal.length === 0 ? (
        <Text size="1" color="gray">
          No rules defined
        </Text>
      ) : (
        <Flex direction="column" gap="2">
          {internal.map((rule) => (
            <Flex key={rule._id} gap="2" align="center" wrap="wrap">
              <TextField.Root
                size="1"
                value={rule.match}
                onChange={(e) => updateRule(rule._id, "match", e.target.value)}
                placeholder="Pattern (e.g., shell:*)"
                style={{ flex: 1, minWidth: 100 }}
              />
              <Flex gap="1" align="center">
                <TextField.Root
                  size="1"
                  value={rule.action}
                  onChange={(e) =>
                    updateRule(rule._id, "action", e.target.value)
                  }
                  placeholder="Action"
                  style={{ width: 70 }}
                />
                <DropdownMenu.Root>
                  <DropdownMenu.Trigger>
                    <IconButton size="1" variant="ghost">
                      <ChevronDownIcon />
                    </IconButton>
                  </DropdownMenu.Trigger>
                  <DropdownMenu.Content>
                    {COMMON_ACTIONS.map((action) => (
                      <DropdownMenu.Item
                        key={action}
                        onSelect={() => updateRule(rule._id, "action", action)}
                      >
                        {action}
                      </DropdownMenu.Item>
                    ))}
                  </DropdownMenu.Content>
                </DropdownMenu.Root>
              </Flex>
              <IconButton
                size="1"
                variant="ghost"
                color="red"
                onClick={() => removeRule(rule._id)}
              >
                <TrashIcon />
              </IconButton>
            </Flex>
          ))}
        </Flex>
      )}
    </Flex>
  );
};

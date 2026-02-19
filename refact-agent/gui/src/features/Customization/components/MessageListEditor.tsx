import React, { useCallback, useState, useEffect } from "react";
import {
  Flex,
  Button,
  TextField,
  IconButton,
  TextArea,
  Text,
  DropdownMenu,
} from "@radix-ui/themes";
import {
  PlusIcon,
  TrashIcon,
  ChevronUpIcon,
  ChevronDownIcon,
  ChevronDownIcon as DropdownIcon,
} from "@radix-ui/react-icons";
import styles from "./editors.module.css";

export type MessageTemplate = {
  role: string;
  content: string;
};

type InternalMessage = MessageTemplate & { _id: string };

type MessageListEditorProps = {
  value: MessageTemplate[];
  onChange: (value: MessageTemplate[]) => void;
  label?: string;
};

const COMMON_ROLES = ["system", "user", "assistant", "tool", "developer"];

let idCounter = 0;
const generateId = () => `msg_${++idCounter}_${Date.now()}`;

const toInternal = (msgs: MessageTemplate[]): InternalMessage[] =>
  msgs.map((m) => ({ ...m, _id: generateId() }));

const toExternal = (msgs: InternalMessage[]): MessageTemplate[] =>
  msgs.map(({ _id, ...rest }) => rest);

export const MessageListEditor: React.FC<MessageListEditorProps> = ({
  value,
  onChange,
  label = "Messages",
}) => {
  const [internal, setInternal] = useState<InternalMessage[]>(() =>
    toInternal(value),
  );
  const valueKey = JSON.stringify(value);

  useEffect(() => {
    setInternal(toInternal(value));
    // eslint-disable-next-line react-hooks/exhaustive-deps -- valueKey is derived from value, used for deep comparison
  }, [valueKey]);

  const emit = useCallback(
    (msgs: InternalMessage[]) => {
      setInternal(msgs);
      onChange(toExternal(msgs));
    },
    [onChange],
  );

  const addMessage = useCallback(() => {
    emit([...internal, { role: "user", content: "", _id: generateId() }]);
  }, [internal, emit]);

  const removeMessage = useCallback(
    (id: string) => {
      emit(internal.filter((m) => m._id !== id));
    },
    [internal, emit],
  );

  const updateMessage = useCallback(
    (id: string, field: keyof MessageTemplate, fieldValue: string) => {
      emit(
        internal.map((m) => (m._id === id ? { ...m, [field]: fieldValue } : m)),
      );
    },
    [internal, emit],
  );

  const moveMessage = useCallback(
    (id: string, direction: -1 | 1) => {
      const idx = internal.findIndex((m) => m._id === id);
      const newIdx = idx + direction;
      if (newIdx < 0 || newIdx >= internal.length) return;
      const newInternal = [...internal];
      [newInternal[idx], newInternal[newIdx]] = [
        newInternal[newIdx],
        newInternal[idx],
      ];
      emit(newInternal);
    },
    [internal, emit],
  );

  return (
    <Flex direction="column" gap="2">
      <Flex justify="between" align="center">
        <Text size="2" weight="medium">
          {label}
        </Text>
        <Button size="1" variant="soft" onClick={addMessage}>
          <PlusIcon /> Add
        </Button>
      </Flex>
      {value.length === 0 && (
        <Text size="1" color="gray">
          No messages defined
        </Text>
      )}
      {internal.map((msg, index) => (
        <Flex
          key={msg._id}
          direction="column"
          gap="2"
          className={styles.messageItem}
        >
          <Flex gap="2" align="center" wrap="wrap">
            <Flex gap="1" align="center">
              <TextField.Root
                size="1"
                value={msg.role}
                onChange={(e) => updateMessage(msg._id, "role", e.target.value)}
                placeholder="Role"
                style={{ width: 90 }}
              />
              <DropdownMenu.Root>
                <DropdownMenu.Trigger>
                  <IconButton size="1" variant="ghost">
                    <DropdownIcon />
                  </IconButton>
                </DropdownMenu.Trigger>
                <DropdownMenu.Content>
                  {COMMON_ROLES.map((role) => (
                    <DropdownMenu.Item
                      key={role}
                      onSelect={() => updateMessage(msg._id, "role", role)}
                    >
                      {role}
                    </DropdownMenu.Item>
                  ))}
                </DropdownMenu.Content>
              </DropdownMenu.Root>
            </Flex>
            <Flex gap="1" ml="auto">
              <IconButton
                size="1"
                variant="ghost"
                disabled={index === 0}
                onClick={() => moveMessage(msg._id, -1)}
              >
                <ChevronUpIcon />
              </IconButton>
              <IconButton
                size="1"
                variant="ghost"
                disabled={index === internal.length - 1}
                onClick={() => moveMessage(msg._id, 1)}
              >
                <ChevronDownIcon />
              </IconButton>
              <IconButton
                size="1"
                variant="ghost"
                color="red"
                onClick={() => removeMessage(msg._id)}
              >
                <TrashIcon />
              </IconButton>
            </Flex>
          </Flex>
          <TextArea
            value={msg.content}
            onChange={(e) => updateMessage(msg._id, "content", e.target.value)}
            placeholder="Message content..."
            rows={2}
            className={styles.messageContent}
          />
        </Flex>
      ))}
    </Flex>
  );
};

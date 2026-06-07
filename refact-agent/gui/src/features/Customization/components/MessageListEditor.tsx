import React, { useCallback, useState, useEffect } from "react";
import { ChevronDown, ChevronUp, Plus, Trash2 } from "lucide-react";

import {
  Button,
  Field,
  FieldSelect,
  FieldText,
  FieldTextarea,
  IconButton,
} from "../../../components/ui";
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
const ROLE_OPTIONS = COMMON_ROLES.map((role) => ({ value: role, label: role }));

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
    <Field label={label}>
      <div className={styles.messageListStack}>
        {value.length === 0 && <p className={styles.emptyText}>No messages defined</p>}
        {internal.map((msg, index) => (
          <div key={msg._id} className={styles.messageItem}>
            <div className={styles.messageToolbar}>
              {COMMON_ROLES.includes(msg.role) ? (
                <FieldSelect
                  options={ROLE_OPTIONS}
                  value={msg.role}
                  onChange={(role) => updateMessage(msg._id, "role", role)}
                />
              ) : (
                <FieldText
                  value={msg.role}
                  onChange={(role) => updateMessage(msg._id, "role", role)}
                  placeholder="Role"
                />
              )}
              <div className={styles.iconGroup}>
                <IconButton
                  aria-label={`Move message ${index + 1} up`}
                  icon={ChevronUp}
                  size="sm"
                  variant="ghost"
                  disabled={index === 0}
                  onClick={() => moveMessage(msg._id, -1)}
                />
                <IconButton
                  aria-label={`Move message ${index + 1} down`}
                  icon={ChevronDown}
                  size="sm"
                  variant="ghost"
                  disabled={index === internal.length - 1}
                  onClick={() => moveMessage(msg._id, 1)}
                />
                <IconButton
                  aria-label={`Remove message ${index + 1}`}
                  icon={Trash2}
                  size="sm"
                  variant="danger"
                  onClick={() => removeMessage(msg._id)}
                />
              </div>
            </div>
            <FieldTextarea
              value={msg.content}
              onChange={(content) => updateMessage(msg._id, "content", content)}
              placeholder="Message content..."
              rows={2}
              className={styles.messageContent}
            />
          </div>
        ))}
        <Button leftIcon={Plus} size="sm" variant="soft" onClick={addMessage}>
          Add
        </Button>
      </div>
    </Field>
  );
};

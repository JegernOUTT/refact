import classNames from "classnames";
import { Plus, Trash2 } from "lucide-react";
import type React from "react";

import { Button, IconButton } from "../Button";
import styles from "./SettingsListEditor.module.css";

export type SettingsListEditorItem = {
  id: string;
  value: string;
};

export type SettingsListEditorProps = {
  items: SettingsListEditorItem[];
  onChange: (id: string, value: string) => void;
  onRemove: (id: string) => void;
  onAdd: () => void;
  addLabel?: string;
  placeholder?: string;
  monospace?: boolean;
  readOnly?: boolean;
  disabled?: boolean;
  emptyLabel?: React.ReactNode;
  itemAriaLabel?: (item: SettingsListEditorItem, index: number) => string;
  className?: string;
};

export function SettingsListEditor({
  addLabel = "Add",
  className,
  disabled = false,
  emptyLabel,
  items,
  itemAriaLabel,
  monospace = false,
  onAdd,
  onChange,
  onRemove,
  placeholder,
  readOnly = false,
}: SettingsListEditorProps) {
  const isEmpty = items.length === 0;

  return (
    <div className={classNames(styles.root, className)}>
      {isEmpty ? (
        emptyLabel ? (
          <p className={styles.empty}>{emptyLabel}</p>
        ) : null
      ) : (
        <div className={styles.rows}>
          {items.map((item, index) => {
            const removeLabel = itemAriaLabel
              ? itemAriaLabel(item, index)
              : `Remove ${item.value}`;

            return (
              <div className={styles.row} key={item.id}>
                <input
                  aria-label={placeholder ?? `Item ${index + 1}`}
                  className={classNames(styles.input, monospace && styles.mono)}
                  disabled={readOnly || disabled}
                  placeholder={placeholder}
                  type="text"
                  value={item.value}
                  onChange={(event) => onChange(item.id, event.target.value)}
                />
                {readOnly ? null : (
                  <IconButton
                    aria-label={removeLabel}
                    className={styles.remove}
                    disabled={disabled}
                    icon={Trash2}
                    size="sm"
                    variant="ghost"
                    onClick={() => onRemove(item.id)}
                  />
                )}
              </div>
            );
          })}
        </div>
      )}
      {readOnly ? null : (
        <Button
          className={styles.addButton}
          disabled={disabled}
          leftIcon={Plus}
          size="sm"
          variant="soft"
          onClick={onAdd}
        >
          {addLabel}
        </Button>
      )}
    </div>
  );
}

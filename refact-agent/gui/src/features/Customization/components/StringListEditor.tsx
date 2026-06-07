import React, { useCallback, useState } from "react";
import { Plus } from "lucide-react";

import { Button, Chip, Field, FieldText } from "../../../components/ui";
import styles from "./editors.module.css";

type StringListEditorProps = {
  value: string[];
  onChange: (value: string[]) => void;
  label?: string;
  placeholder?: string;
  suggestions?: string[];
};

export const StringListEditor: React.FC<StringListEditorProps> = ({
  value,
  onChange,
  label = "Items",
  placeholder = "Add item...",
  suggestions = [],
}) => {
  const [inputValue, setInputValue] = useState("");
  const [showSuggestions, setShowSuggestions] = useState(false);

  const addItem = useCallback(
    (item: string) => {
      const trimmed = item.trim();
      if (trimmed && !value.includes(trimmed)) {
        onChange([...value, trimmed]);
      }
      setInputValue("");
      setShowSuggestions(false);
    },
    [value, onChange],
  );

  const removeItem = useCallback(
    (index: number) => {
      onChange(value.filter((_, i) => i !== index));
    },
    [value, onChange],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        addItem(inputValue);
      }
    },
    [inputValue, addItem],
  );

  const filteredSuggestions = suggestions
    .filter(
      (s) =>
        !value.includes(s) &&
        s.toLowerCase().includes(inputValue.toLowerCase()),
    )
    .slice(0, 10);

  return (
    <Field label={label}>
      <div className={styles.stringListStack}>
        <div className={styles.chipList}>
          {value.map((item, index) => (
            <Chip key={index} removable radius="chip" onRemove={() => removeItem(index)}>
              {item}
            </Chip>
          ))}
        </div>
        <div className={styles.addRow}>
          <FieldText
            value={inputValue}
            onChange={(nextValue) => {
              setInputValue(nextValue);
              setShowSuggestions(true);
            }}
            onKeyDown={handleKeyDown}
            onFocus={() => setShowSuggestions(true)}
            onBlur={() => setTimeout(() => setShowSuggestions(false), 200)}
            placeholder={placeholder}
          />
          <Button
            aria-label={`Add ${label}`}
            leftIcon={Plus}
            size="sm"
            variant="soft"
            onClick={() => addItem(inputValue)}
            disabled={!inputValue.trim()}
          >
            Add
          </Button>
          {showSuggestions && filteredSuggestions.length > 0 && (
            <div className={styles.suggestions}>
              {filteredSuggestions.map((suggestion) => (
                <button
                  key={suggestion}
                  type="button"
                  className={styles.suggestionItem}
                  onMouseDown={() => addItem(suggestion)}
                >
                  {suggestion}
                </button>
              ))}
            </div>
          )}
        </div>
      </div>
    </Field>
  );
};

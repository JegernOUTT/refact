import React, { useCallback, useState } from "react";
import {
  Flex,
  Button,
  TextField,
  IconButton,
  Text,
  Badge,
} from "@radix-ui/themes";
import { PlusIcon, Cross2Icon } from "@radix-ui/react-icons";
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
    <Flex direction="column" gap="2">
      <Text size="2" weight="medium">
        {label}
      </Text>
      <Flex gap="1" wrap="wrap">
        {value.map((item, index) => (
          <Badge
            key={index}
            size="2"
            variant="soft"
            className={styles.tagBadge}
          >
            {item}
            <IconButton
              size="1"
              variant="ghost"
              className={styles.tagRemove}
              onClick={() => removeItem(index)}
            >
              <Cross2Icon width={10} height={10} />
            </IconButton>
          </Badge>
        ))}
      </Flex>
      <Flex gap="2" style={{ position: "relative" }}>
        <TextField.Root
          value={inputValue}
          onChange={(e) => {
            setInputValue(e.target.value);
            setShowSuggestions(true);
          }}
          onKeyDown={handleKeyDown}
          onFocus={() => setShowSuggestions(true)}
          onBlur={() => setTimeout(() => setShowSuggestions(false), 200)}
          placeholder={placeholder}
          style={{ flex: 1 }}
        />
        <Button
          size="2"
          variant="soft"
          onClick={() => addItem(inputValue)}
          disabled={!inputValue.trim()}
        >
          <PlusIcon />
        </Button>
        {showSuggestions && filteredSuggestions.length > 0 && (
          <Flex direction="column" className={styles.suggestions}>
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
          </Flex>
        )}
      </Flex>
    </Flex>
  );
};

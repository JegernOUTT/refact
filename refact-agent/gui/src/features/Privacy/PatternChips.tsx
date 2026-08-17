import { useState } from "react";
import { Plus } from "lucide-react";

import { Button, Chip, FieldText } from "../../components/ui";
import styles from "./PrivacySettingsSection.module.css";

export interface PatternChipsProps {
  patterns: string[];
  disabled?: boolean;
  addLabel?: string;
  emptyLabel?: string;
  placeholder?: string;
  onChange: (patterns: string[]) => void;
}

export function PatternChips({
  addLabel = "Add pattern",
  disabled = false,
  emptyLabel = "No patterns yet",
  onChange,
  patterns,
  placeholder = "e.g. .env*",
}: PatternChipsProps) {
  const [draft, setDraft] = useState("");
  const [adding, setAdding] = useState(false);

  const commit = () => {
    const value = draft.trim();
    setDraft("");
    setAdding(false);
    if (value.length === 0 || patterns.includes(value)) return;
    onChange([...patterns, value]);
  };

  return (
    <div className={styles.chipRow}>
      {patterns.length === 0 && !adding ? (
        <span className={styles.emptyHint}>{emptyLabel}</span>
      ) : null}
      {patterns.map((pattern, index) => (
        <Chip
          className={styles.patternChip}
          disabled={disabled}
          key={`${pattern}-${String(index)}`}
          radius="chip"
          removable
          onRemove={() => onChange(patterns.filter((_, at) => at !== index))}
        >
          {pattern}
        </Chip>
      ))}
      {adding ? (
        <FieldText
          aria-label={addLabel}
          autoFocus
          className={styles.patternInput}
          disabled={disabled}
          placeholder={placeholder}
          value={draft}
          onBlur={commit}
          onChange={setDraft}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              commit();
            }
            if (event.key === "Escape") {
              event.preventDefault();
              setDraft("");
              setAdding(false);
            }
          }}
        />
      ) : (
        <Button
          disabled={disabled}
          leftIcon={Plus}
          size="sm"
          variant="ghost"
          onClick={() => setAdding(true)}
        >
          {addLabel}
        </Button>
      )}
    </div>
  );
}

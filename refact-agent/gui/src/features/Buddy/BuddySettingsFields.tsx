import React, { useEffect, useRef, useState } from "react";
import { FieldSwitch, FieldText, SettingItem } from "../../components/ui";
import styles from "./BuddySettingsPanel.module.css";

interface SwitchRowProps {
  title: string;
  description?: React.ReactNode;
  checked: boolean;
  onChange: (checked: boolean) => void;
  ariaLabel?: string;
  disabled?: boolean;
  testId?: string;
}

export const SwitchRow: React.FC<SwitchRowProps> = ({
  title,
  description,
  checked,
  onChange,
  ariaLabel,
  disabled,
  testId,
}) => (
  <SettingItem
    className={styles.inlineRow}
    title={title}
    description={description}
    control={
      <FieldSwitch
        checked={checked}
        onChange={onChange}
        aria-label={ariaLabel ?? title}
        disabled={disabled}
        data-testid={testId}
      />
    }
  />
);

interface CommitNumberFieldProps {
  value: number | null;
  onCommit: (raw: string) => boolean;
  ariaLabel: string;
  placeholder?: string;
  min?: number;
  max?: number;
  testId?: string;
}

export const CommitNumberField: React.FC<CommitNumberFieldProps> = ({
  value,
  onCommit,
  ariaLabel,
  placeholder,
  min,
  max,
  testId,
}) => {
  const live = value === null ? "" : String(value);
  const [draft, setDraft] = useState(live);
  const editingRef = useRef(false);
  const pendingRef = useRef<string | null>(null);
  const commitRef = useRef(onCommit);
  commitRef.current = onCommit;

  useEffect(
    () => () => {
      const pending = pendingRef.current;
      pendingRef.current = null;
      if (pending !== null) commitRef.current(pending);
    },
    [],
  );

  useEffect(() => {
    if (!editingRef.current) setDraft(live);
  }, [live]);

  const commit = (input: HTMLInputElement) => {
    editingRef.current = false;
    pendingRef.current = null;
    const raw = input.value;
    if (input.validity.badInput || (raw !== live && !onCommit(raw))) {
      setDraft(live);
    }
  };

  return (
    <FieldText
      type="number"
      inputMode="numeric"
      className={styles.numberField}
      min={min}
      max={max}
      value={draft}
      placeholder={placeholder}
      aria-label={ariaLabel}
      data-testid={testId}
      onChange={(next) => {
        editingRef.current = true;
        const number = Number(next);
        pendingRef.current =
          next !== live &&
          (next === "" ||
            (Number.isFinite(number) &&
              (min === undefined || number >= min) &&
              (max === undefined || number <= max)))
            ? next
            : null;
        setDraft(next);
      }}
      onBlur={(event) => commit(event.currentTarget)}
      onKeyDown={(event) => {
        if (event.key === "Enter") event.currentTarget.blur();
        if (event.key === "Escape") {
          editingRef.current = false;
          pendingRef.current = null;
          setDraft(live);
        }
      }}
    />
  );
};

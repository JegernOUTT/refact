import React, { useState, useCallback, useRef, useEffect } from "react";
import { ExternalLink, Eye, EyeOff, X } from "lucide-react";

import {
  FieldText,
  FieldTextarea,
  Icon,
  IconButton,
  SaveStatus,
  SettingItem,
  Switch,
} from "../../../components/ui";

import styles from "./ProviderForm.module.css";

export type SchemaFieldDef = {
  key: string;
  f_type: string;
  f_desc?: string;
  f_label?: string;
  f_placeholder?: string;
  f_default?: string;
  f_extra?: boolean;
  f_secret?: boolean;
  smartlinks?: { sl_label: string; sl_goto: string }[];
};

type FieldSaveState = "idle" | "saving" | "saved" | "error";

export type SchemaFieldProps = {
  field: SchemaFieldDef;
  value: unknown;
  disabled?: boolean;
  onSave: (key: string, value: unknown) => Promise<void>;
};

export const SchemaField: React.FC<SchemaFieldProps> = ({
  field,
  value,
  disabled = false,
  onSave,
}) => {
  const isSecret =
    field.f_secret === true ||
    field.key.toLowerCase().includes("key") ||
    field.key.toLowerCase().includes("token") ||
    field.key.toLowerCase().includes("secret");

  if (field.f_type === "boolean") {
    return (
      <BooleanField
        field={field}
        value={value}
        disabled={disabled}
        onSave={onSave}
      />
    );
  }

  if (isSecret) {
    return (
      <SecretField
        field={field}
        value={value}
        disabled={disabled}
        onSave={onSave}
      />
    );
  }

  if (field.f_type === "integer" || field.f_type === "number") {
    return (
      <NumberField
        field={field}
        value={value}
        disabled={disabled}
        onSave={onSave}
      />
    );
  }

  return (
    <StringField
      field={field}
      value={value}
      disabled={disabled}
      onSave={onSave}
    />
  );
};

function FieldActions({ field }: { field: SchemaFieldDef }) {
  if (!field.smartlinks?.length) return null;

  return (
    <div className={styles.fieldActions}>
      {field.smartlinks.map((link) => (
        <a
          key={link.sl_goto}
          href={link.sl_goto}
          target="_blank"
          rel="noopener noreferrer"
          className={styles.smartlink}
        >
          {link.sl_label}
          <Icon icon={ExternalLink} size="sm" tone="muted" />
        </a>
      ))}
    </div>
  );
}

function resetStatusLater(
  timerRef: React.MutableRefObject<ReturnType<typeof setTimeout> | undefined>,
  setSaveState: React.Dispatch<React.SetStateAction<FieldSaveState>>,
  state: FieldSaveState,
) {
  timerRef.current = setTimeout(
    () => setSaveState("idle"),
    state === "saved" ? 1500 : 2000,
  );
}

const NumberField: React.FC<SchemaFieldProps> = ({
  field,
  value,
  disabled,
  onSave,
}) => {
  const valueToString = useCallback(
    (candidate: unknown) => String(candidate ?? field.f_default ?? ""),
    [field.f_default],
  );
  const [localValue, setLocalValue] = useState(valueToString(value));
  const [saveState, setSaveState] = useState<FieldSaveState>("idle");
  const originalValueRef = useRef(value);
  const timerRef = useRef<ReturnType<typeof setTimeout>>();
  useEffect(() => () => clearTimeout(timerRef.current), []);

  useEffect(() => {
    originalValueRef.current = value;
    setLocalValue(valueToString(value));
  }, [value, valueToString]);

  const handleBlur = useCallback(async () => {
    if (localValue === valueToString(originalValueRef.current)) return;
    const parsed = Number(localValue);
    if (!Number.isFinite(parsed)) {
      setSaveState("error");
      resetStatusLater(timerRef, setSaveState, "error");
      return;
    }
    const nextValue = field.f_type === "integer" ? Math.trunc(parsed) : parsed;
    setSaveState("saving");
    try {
      await onSave(field.key, nextValue);
      setSaveState("saved");
      resetStatusLater(timerRef, setSaveState, "saved");
    } catch {
      setSaveState("error");
      resetStatusLater(timerRef, setSaveState, "error");
    }
  }, [field.f_type, field.key, localValue, onSave, valueToString]);

  return (
    <SettingItem
      title={field.f_label ?? field.key}
      description={field.f_desc}
      layout="stack"
      saveStatus={saveState}
      control={
        <FieldText
          id={field.key}
          type="number"
          value={localValue}
          placeholder={field.f_placeholder ?? ""}
          disabled={disabled}
          onChange={setLocalValue}
          onBlur={() => void handleBlur()}
          onKeyDown={(event) => {
            if (event.key === "Enter") event.currentTarget.blur();
          }}
        />
      }
    />
  );
};

const BooleanField: React.FC<SchemaFieldProps> = ({
  field,
  value,
  disabled,
  onSave,
}) => {
  const [saveState, setSaveState] = useState<FieldSaveState>("idle");
  const timerRef = useRef<ReturnType<typeof setTimeout>>();
  useEffect(() => () => clearTimeout(timerRef.current), []);

  const handleChange = useCallback(
    async (checked: boolean) => {
      setSaveState("saving");
      try {
        await onSave(field.key, checked);
        setSaveState("saved");
        resetStatusLater(timerRef, setSaveState, "saved");
      } catch {
        setSaveState("error");
        resetStatusLater(timerRef, setSaveState, "error");
      }
    },
    [field.key, onSave],
  );

  return (
    <SettingItem
      title={field.f_label ?? field.key}
      description={field.f_desc}
      saveStatus={saveState}
      control={
        <Switch
          id={field.key}
          checked={Boolean(value)}
          disabled={disabled}
          onCheckedChange={(checked) => void handleChange(checked)}
        />
      }
    />
  );
};

const SecretField: React.FC<SchemaFieldProps> = ({
  field,
  value,
  disabled,
  onSave,
}) => {
  const isMasked = value === "***";
  const [localValue, setLocalValue] = useState("");
  const [revealed, setRevealed] = useState(false);
  const [saveState, setSaveState] = useState<FieldSaveState>("idle");
  const [editing, setEditing] = useState(false);
  const originalValueRef = useRef(value);
  const timerRef = useRef<ReturnType<typeof setTimeout>>();
  useEffect(() => () => clearTimeout(timerRef.current), []);

  useEffect(() => {
    originalValueRef.current = value;
    if (!editing) {
      setLocalValue("");
    }
  }, [value, editing]);

  const handleBlur = useCallback(async () => {
    if (!editing) return;
    if (localValue === "" && isMasked) {
      setEditing(false);
      return;
    }
    if (localValue === String(originalValueRef.current)) {
      setEditing(false);
      return;
    }
    setSaveState("saving");
    try {
      await onSave(field.key, localValue);
      setSaveState("saved");
      setEditing(false);
      resetStatusLater(timerRef, setSaveState, "saved");
    } catch {
      setSaveState("error");
      resetStatusLater(timerRef, setSaveState, "error");
    }
  }, [editing, localValue, isMasked, field.key, onSave]);

  const handleClear = useCallback(async () => {
    setSaveState("saving");
    try {
      await onSave(field.key, "");
      setLocalValue("");
      setEditing(false);
      setSaveState("saved");
      resetStatusLater(timerRef, setSaveState, "saved");
    } catch {
      setSaveState("error");
      resetStatusLater(timerRef, setSaveState, "error");
    }
  }, [field.key, onSave]);

  const displayValue = editing
    ? localValue
    : isMasked
      ? ""
      : String(value ?? "");
  const placeholder =
    isMasked && !editing ? "••••••••  (saved)" : field.f_placeholder ?? "";

  return (
    <SettingItem
      title={field.f_label ?? field.key}
      description={field.f_desc}
      layout="stack"
      saveStatus={saveState}
      control={
        <div className={styles.fieldControlStack}>
          <div className={styles.fieldControlRow}>
            <FieldText
              id={field.key}
              type={revealed ? "text" : "password"}
              value={displayValue}
              placeholder={placeholder}
              disabled={disabled}
              className={styles.fieldGrow}
              onFocus={() => setEditing(true)}
              onChange={setLocalValue}
              onBlur={() => void handleBlur()}
              onKeyDown={(event) => {
                if (event.key === "Enter") event.currentTarget.blur();
              }}
            />
            <IconButton
              aria-label={revealed ? "Hide" : "Reveal"}
              icon={revealed ? EyeOff : Eye}
              variant="ghost"
              size="sm"
              onClick={() => setRevealed(!revealed)}
            />
            {isMasked && !editing ? (
              <IconButton
                aria-label="Clear saved value"
                icon={X}
                variant="danger"
                size="sm"
                onClick={() => void handleClear()}
              />
            ) : null}
          </div>
          <FieldActions field={field} />
        </div>
      }
    />
  );
};

const StringField: React.FC<SchemaFieldProps> = ({
  field,
  value,
  disabled,
  onSave,
}) => {
  const [localValue, setLocalValue] = useState(
    String(value ?? field.f_default ?? ""),
  );
  const [saveState, setSaveState] = useState<FieldSaveState>("idle");
  const originalValueRef = useRef(value);
  const timerRef = useRef<ReturnType<typeof setTimeout>>();
  useEffect(() => () => clearTimeout(timerRef.current), []);

  useEffect(() => {
    originalValueRef.current = value;
    setLocalValue(String(value ?? field.f_default ?? ""));
  }, [value, field.f_default]);

  const handleBlur = useCallback(async () => {
    if (localValue === String(originalValueRef.current ?? "")) return;
    setSaveState("saving");
    try {
      await onSave(field.key, localValue);
      setSaveState("saved");
      resetStatusLater(timerRef, setSaveState, "saved");
    } catch {
      setSaveState("error");
      resetStatusLater(timerRef, setSaveState, "error");
    }
  }, [localValue, field.key, onSave]);

  const isLong = field.f_type === "string_long" || localValue.length > 80;

  return (
    <SettingItem
      title={field.f_label ?? field.key}
      description={field.f_desc}
      layout="stack"
      saveStatus={saveState}
      control={
        <div className={styles.fieldControlStack}>
          {isLong ? (
            <FieldTextarea
              id={field.key}
              value={localValue}
              placeholder={field.f_placeholder ?? ""}
              disabled={disabled}
              onChange={setLocalValue}
              onBlur={() => void handleBlur()}
              rows={2}
            />
          ) : (
            <FieldText
              id={field.key}
              value={localValue}
              placeholder={field.f_placeholder ?? ""}
              disabled={disabled}
              onChange={setLocalValue}
              onBlur={() => void handleBlur()}
              onKeyDown={(event) => {
                if (event.key === "Enter") event.currentTarget.blur();
              }}
            />
          )}
          <FieldActions field={field} />
        </div>
      }
    />
  );
};

export const SaveIndicator: React.FC<{ state: FieldSaveState }> = ({
  state,
}) => {
  if (state === "idle") return null;

  return <SaveStatus state={state} />;
};

import React, { useState, useCallback, useRef, useEffect } from "react";
import {
  Button,
  Flex,
  Switch,
  Text,
  TextField,
  TextArea,
} from "@radix-ui/themes";
import {
  ExternalLinkIcon,
  EyeOpenIcon,
  EyeClosedIcon,
  Cross2Icon,
  CheckIcon,
} from "@radix-ui/react-icons";
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

  return (
    <StringField
      field={field}
      value={value}
      disabled={disabled}
      onSave={onSave}
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
        timerRef.current = setTimeout(() => setSaveState("idle"), 1500);
      } catch {
        setSaveState("error");
        timerRef.current = setTimeout(() => setSaveState("idle"), 2000);
      }
    },
    [field.key, onSave],
  );

  return (
    <Flex direction="column" gap="1">
      <Flex align="center" justify="between" gap="3">
        <Flex direction="column" gap="0">
          <label htmlFor={field.key}>
            <Text size="2" weight="medium">
              {field.f_label ?? field.key}
            </Text>
          </label>
          {field.f_desc && (
            <Text size="1" color="gray">
              {field.f_desc}
            </Text>
          )}
        </Flex>
        <Flex align="center" gap="2">
          <SaveIndicator state={saveState} />
          <Switch
            id={field.key}
            checked={Boolean(value)}
            disabled={disabled}
            onCheckedChange={(checked) => void handleChange(checked)}
          />
        </Flex>
      </Flex>
    </Flex>
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
      timerRef.current = setTimeout(() => setSaveState("idle"), 1500);
    } catch {
      setSaveState("error");
      timerRef.current = setTimeout(() => setSaveState("idle"), 2000);
    }
  }, [editing, localValue, isMasked, field.key, onSave]);

  const handleClear = useCallback(async () => {
    setSaveState("saving");
    try {
      await onSave(field.key, "");
      setLocalValue("");
      setEditing(false);
      setSaveState("saved");
      timerRef.current = setTimeout(() => setSaveState("idle"), 1500);
    } catch {
      setSaveState("error");
      timerRef.current = setTimeout(() => setSaveState("idle"), 2000);
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
    <Flex direction="column" gap="1">
      <Flex align="center" justify="between">
        <Flex direction="column" gap="0">
          <Text size="2" weight="medium">
            {field.f_label ?? field.key}
          </Text>
          {field.f_desc && (
            <Text size="1" color="gray">
              {field.f_desc}
            </Text>
          )}
        </Flex>
        <Flex align="center" gap="1">
          <SaveIndicator state={saveState} />
          {field.smartlinks?.map((link) => (
            <Button key={link.sl_goto} variant="ghost" size="1" asChild>
              <a href={link.sl_goto} target="_blank" rel="noopener noreferrer">
                <ExternalLinkIcon width={12} height={12} />
                <Text size="1">{link.sl_label}</Text>
              </a>
            </Button>
          ))}
        </Flex>
      </Flex>
      <Flex gap="1" align="center">
        <TextField.Root
          style={{ flex: 1 }}
          id={field.key}
          type={revealed ? "text" : "password"}
          value={displayValue}
          placeholder={placeholder}
          disabled={disabled}
          onFocus={() => setEditing(true)}
          onChange={(e) => setLocalValue(e.target.value)}
          onBlur={() => void handleBlur()}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
          }}
          className={disabled ? styles.disabledField : undefined}
        />
        <Button
          variant="ghost"
          size="1"
          onClick={() => setRevealed(!revealed)}
          title={revealed ? "Hide" : "Reveal"}
        >
          {revealed ? <EyeClosedIcon /> : <EyeOpenIcon />}
        </Button>
        {isMasked && !editing && (
          <Button
            variant="ghost"
            size="1"
            color="red"
            onClick={() => void handleClear()}
            title="Clear saved value"
          >
            <Cross2Icon />
          </Button>
        )}
      </Flex>
    </Flex>
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
      timerRef.current = setTimeout(() => setSaveState("idle"), 1500);
    } catch {
      setSaveState("error");
      timerRef.current = setTimeout(() => setSaveState("idle"), 2000);
    }
  }, [localValue, field.key, onSave]);

  const isLong = field.f_type === "string_long" || localValue.length > 80;

  return (
    <Flex direction="column" gap="1">
      <Flex align="center" justify="between">
        <Flex direction="column" gap="0">
          <Text size="2" weight="medium">
            {field.f_label ?? field.key}
          </Text>
          {field.f_desc && (
            <Text size="1" color="gray">
              {field.f_desc}
            </Text>
          )}
        </Flex>
        <Flex align="center" gap="1">
          <SaveIndicator state={saveState} />
          {field.smartlinks?.map((link) => (
            <Button key={link.sl_goto} variant="ghost" size="1" asChild>
              <a href={link.sl_goto} target="_blank" rel="noopener noreferrer">
                <ExternalLinkIcon width={12} height={12} />
                <Text size="1">{link.sl_label}</Text>
              </a>
            </Button>
          ))}
        </Flex>
      </Flex>
      {isLong ? (
        <TextArea
          id={field.key}
          value={localValue}
          placeholder={field.f_placeholder ?? ""}
          disabled={disabled}
          onChange={(e) => setLocalValue(e.target.value)}
          onBlur={() => void handleBlur()}
          className={disabled ? styles.disabledField : undefined}
          rows={2}
        />
      ) : (
        <TextField.Root
          id={field.key}
          value={localValue}
          placeholder={field.f_placeholder ?? ""}
          disabled={disabled}
          onChange={(e) => setLocalValue(e.target.value)}
          onBlur={() => void handleBlur()}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
          }}
          className={disabled ? styles.disabledField : undefined}
        />
      )}
    </Flex>
  );
};

const SaveIndicator: React.FC<{ state: FieldSaveState }> = ({ state }) => {
  switch (state) {
    case "idle":
      return null;
    case "saving":
      return (
        <Text size="1" color="gray">
          Saving…
        </Text>
      );
    case "saved":
      return (
        <Flex align="center" gap="1">
          <CheckIcon width={12} height={12} color="var(--green-9)" />
          <Text size="1" color="green">
            Saved
          </Text>
        </Flex>
      );
    case "error":
      return (
        <Text size="1" color="red">
          Error
        </Text>
      );
  }
};

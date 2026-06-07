import { Markdown } from "../Markdown";
import { useCallback, useEffect, useRef, useState } from "react";
import { FieldSwitch, FieldText, FieldTextarea } from "../ui";
import styles from "./CustomFieldsAndWidgets.module.css";

export const CustomInputField = ({
  value,
  placeholder,
  type,
  id,
  name,
  size = "long",
  onChange,
  wasInteracted = false,
}: {
  id?: string;
  wasInteracted?: boolean;
  type?:
    | "number"
    | "search"
    | "time"
    | "text"
    | "hidden"
    | "tel"
    | "url"
    | "email"
    | "date"
    | "password"
    | "datetime-local"
    | "month"
    | "week";
  value?: string;
  name?: string;
  placeholder?: string;
  size?: string;
  width?: string;
  color?: string;
  onChange?: (value: string) => void;
}) => {
  const wasInitialized = useRef(wasInteracted);

  useEffect(() => {
    if (!wasInitialized.current && onChange) {
      onChange(value ?? "");
      wasInitialized.current = true;
    }
  }, [onChange, value]);

  return (
    <div className={styles.fieldWrap}>
      {size !== "multiline" ? (
        <FieldText
          id={id}
          name={name}
          type={type}
          value={value ?? ""}
          placeholder={placeholder}
          onChange={(nextValue) => onChange?.(nextValue)}
        />
      ) : (
        <FieldTextarea
          id={id}
          name={name}
          rows={3}
          value={value ?? ""}
          placeholder={placeholder}
          onChange={(nextValue) => onChange?.(nextValue)}
        />
      )}
    </div>
  );
};

export const CustomLabel = ({
  label,
  htmlFor,
}: {
  label: string;
  htmlFor?: string;
  mt?: string;
}) => {
  return (
    <span className={styles.label}>
      <label htmlFor={htmlFor}>{label}</label>
    </span>
  );
};

export const CustomDescriptionField = ({
  children = "",
}: {
  children?: string;
  mb?: string;
}) => {
  return (
    <span className={styles.description}>
      <Markdown>{children}</Markdown>
    </span>
  );
};

export const CustomBoolField = ({
  id,
  name,
  value,
  onChange,
}: {
  id: string;
  name: string;
  value: boolean;
  onChange: (value: boolean) => void;
}) => {
  const [checked, setChecked] = useState(value);

  const onCheckedChange = useCallback(
    (value: boolean) => {
      setChecked(value);
      onChange(value);
    },
    [onChange],
  );

  return (
    <div>
      <FieldSwitch name={name} id={id} checked={checked} onChange={onCheckedChange} />
      <input type="hidden" name={name} value={checked ? "on" : "off"} />
    </div>
  );
};

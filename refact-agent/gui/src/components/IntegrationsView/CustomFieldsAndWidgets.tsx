import { Box, Text } from "@radix-ui/themes";
import { Markdown } from "../Markdown";
import { useCallback, useEffect, useRef, useState } from "react";
import { FieldSwitch, FieldText, FieldTextarea } from "../ui";

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
    <Box width="100%">
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
    </Box>
  );
};

export const CustomLabel = ({
  label,
  htmlFor,
  mt,
}: {
  label: string;
  htmlFor?: string;
  mt?: string;
}) => {
  return (
    <Text
      as="label"
      htmlFor={htmlFor}
      size="2"
      weight="medium"
      mt={mt ? mt : "0"}
      style={{
        display: "block",
      }}
    >
      {label}
    </Text>
  );
};

export const CustomDescriptionField = ({
  children = "",
  mb = "2",
}: {
  children?: string;
  mb?: string;
}) => {
  return (
    <Text
      size="1"
      mb={{
        initial: "0",
        xs: mb,
      }}
      style={{ display: "block", opacity: 0.85 }}
    >
      <Markdown>{children}</Markdown>
    </Text>
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
    <Box>
      <FieldSwitch name={name} id={id} checked={checked} onChange={onCheckedChange} />
      <input type="hidden" name={name} value={checked ? "on" : "off"} />
    </Box>
  );
};

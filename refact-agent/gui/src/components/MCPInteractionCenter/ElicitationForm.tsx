import React, { useEffect, useMemo, useState } from "react";

import type {
  MCPElicitationSchema,
  MCPElicitationSchemaProperty,
} from "../../services/refact/mcpInteractions";
import {
  Button,
  Field,
  FieldSelect,
  FieldSwitch,
  FieldText,
  FieldTextarea,
  Flex,
} from "../ui";
import styles from "./MCPInteractionCenter.module.css";

type ElicitationValue = string | number | boolean;

type ElicitationFormProps = {
  schema: MCPElicitationSchema;
  disabled?: boolean;
  onCancel: () => void;
  onDecline: () => void;
  onSubmit: (content: Record<string, ElicitationValue>) => void;
};

const EMPTY_PROPERTIES: NonNullable<MCPElicitationSchema["properties"]> = {};

function isNumberProperty(property: MCPElicitationSchemaProperty) {
  return property.type === "number" || property.type === "integer";
}

function isBooleanProperty(property: MCPElicitationSchemaProperty) {
  return property.type === "boolean";
}

function getInitialValue(
  property: MCPElicitationSchemaProperty,
): ElicitationValue {
  if (property.default !== undefined) return property.default;
  if (property.enum?.length) return property.enum[0];
  if (isBooleanProperty(property)) return false;
  return "";
}

function parseValue(
  value: ElicitationValue,
  property: MCPElicitationSchemaProperty,
): ElicitationValue {
  if (isBooleanProperty(property)) return Boolean(value);

  if (isNumberProperty(property)) {
    if (value === "") return "";
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : "";
  }

  return String(value);
}

function hasRequiredValue(value: ElicitationValue | undefined) {
  if (typeof value === "boolean") return true;
  if (typeof value === "number") return Number.isFinite(value);
  return typeof value === "string" && value.trim().length > 0;
}

export function ElicitationForm({
  disabled = false,
  onCancel,
  onDecline,
  onSubmit,
  schema,
}: ElicitationFormProps) {
  const properties = schema.properties ?? EMPTY_PROPERTIES;
  const required = useMemo(
    () => new Set(schema.required ?? []),
    [schema.required],
  );

  const initialValues = useMemo(() => {
    return Object.fromEntries(
      Object.entries(properties).map(([key, property]) => [
        key,
        getInitialValue(property),
      ]),
    ) as Record<string, ElicitationValue>;
  }, [properties]);

  const [values, setValues] =
    useState<Record<string, ElicitationValue>>(initialValues);

  useEffect(() => {
    setValues(initialValues);
  }, [initialValues]);

  const isValid = Array.from(required).every((key) =>
    hasRequiredValue(values[key]),
  );

  const handleSubmit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!isValid || disabled) return;

    const content = Object.fromEntries(
      Object.entries(properties).map(([key, property]) => [
        key,
        parseValue(
          (values as Record<string, ElicitationValue | undefined>)[key] ?? "",
          property,
        ),
      ]),
    ) as Record<string, ElicitationValue>;

    onSubmit(content);
  };

  return (
    <form className={styles.form} onSubmit={handleSubmit}>
      <Flex direction="column" gap="3">
        {Object.entries(properties).map(([key, property]) => {
          const id = `mcp-elicitation-${key}`;
          const label = property.title ?? key;
          const helper = property.description;
          const value =
            (values as Record<string, ElicitationValue | undefined>)[key] ??
            getInitialValue(property);

          if (property.enum?.length) {
            const options = property.enum.map((option, index) => ({
              value: option,
              label: property.enumNames?.[index] ?? option,
            }));

            return (
              <Field
                key={key}
                htmlFor={id}
                label={label}
                helper={helper}
                required={required.has(key)}
              >
                <FieldSelect
                  id={id}
                  value={String(value)}
                  options={options}
                  disabled={disabled}
                  onChange={(nextValue) =>
                    setValues((previous) => ({ ...previous, [key]: nextValue }))
                  }
                />
              </Field>
            );
          }

          if (isBooleanProperty(property)) {
            return (
              <Field
                key={key}
                htmlFor={id}
                label={label}
                helper={helper}
                required={required.has(key)}
              >
                <FieldSwitch
                  id={id}
                  checked={Boolean(value)}
                  disabled={disabled}
                  onChange={(nextValue) =>
                    setValues((previous) => ({ ...previous, [key]: nextValue }))
                  }
                />
              </Field>
            );
          }

          if (property.maxLength !== undefined && property.maxLength > 200) {
            return (
              <Field
                key={key}
                htmlFor={id}
                label={label}
                helper={helper}
                required={required.has(key)}
              >
                <FieldTextarea
                  id={id}
                  value={String(value)}
                  rows={4}
                  disabled={disabled}
                  maxLength={property.maxLength}
                  onChange={(nextValue) =>
                    setValues((previous) => ({ ...previous, [key]: nextValue }))
                  }
                />
              </Field>
            );
          }

          return (
            <Field
              key={key}
              htmlFor={id}
              label={label}
              helper={helper}
              required={required.has(key)}
            >
              <FieldText
                id={id}
                type={isNumberProperty(property) ? "number" : "text"}
                inputMode={isNumberProperty(property) ? "numeric" : undefined}
                value={String(value)}
                disabled={disabled}
                min={property.minimum}
                max={property.maximum}
                maxLength={property.maxLength}
                onChange={(nextValue) =>
                  setValues((previous) => ({ ...previous, [key]: nextValue }))
                }
              />
            </Field>
          );
        })}
      </Flex>

      <Flex className={styles.actions} gap="2" justify="between" wrap="wrap">
        <Button
          type="button"
          variant="plain"
          disabled={disabled}
          onClick={onCancel}
        >
          Cancel operation
        </Button>
        <Flex gap="2" justify="end" wrap="wrap">
          <Button
            type="button"
            variant="ghost"
            disabled={disabled}
            onClick={onDecline}
          >
            Decline
          </Button>
          <Button
            type="submit"
            variant="primary"
            disabled={!isValid || disabled}
          >
            Submit
          </Button>
        </Flex>
      </Flex>
    </form>
  );
}

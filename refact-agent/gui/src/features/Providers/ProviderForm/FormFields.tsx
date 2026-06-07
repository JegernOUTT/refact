import { type FC } from "react";

import { FieldSelect, FieldStack, FieldText } from "../../../components/ui";
import { toPascalCase } from "../../../utils/toPascalCase";

import type { ProviderFormValues } from "./useProviderForm";

export type FormFieldsProps = {
  providerData: ProviderFormValues;
  fields: Record<string, string | boolean>;
  onChange: (updatedProviderData: ProviderFormValues) => void;
};

export const FormFields: FC<FormFieldsProps> = ({ providerData, fields, onChange }) => {
  return Object.entries(fields).map(([key, value], idx) => {
    if (key === "endpoint_style") {
      return (
        <FieldStack
          key={`${key}_${idx}`}
          label={toPascalCase(key)}
          control={
            <FieldSelect
              value={value.toString()}
              options={[
                { value: "openai", label: "OpenAI" },
                { value: "hf", label: "HuggingFace" },
              ]}
              onChange={(newValue) => onChange({ ...providerData, endpoint_style: newValue })}
              disabled={providerData.readonly}
            />
          }
        />
      );
    }

    return (
      <FieldStack
        key={`${key}_${idx}`}
        label={toPascalCase(key)}
        htmlFor={key}
        control={
          <FieldText
            id={key}
            value={value.toString()}
            onChange={(newValue) => onChange({ ...providerData, [key]: newValue })}
            disabled={providerData.readonly}
          />
        }
      />
    );
  });
};

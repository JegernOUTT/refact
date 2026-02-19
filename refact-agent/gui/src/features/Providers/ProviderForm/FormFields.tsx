import { FC } from "react";
import classNames from "classnames";

import { Flex, Select, TextField } from "@radix-ui/themes";
import { toPascalCase } from "../../../utils/toPascalCase";

import type { ProviderFormValues } from "./useProviderForm";

import styles from "./ProviderForm.module.css";

export type FormFieldsProps = {
  providerData: ProviderFormValues;
  fields: Record<string, string | boolean>;
  onChange: (updatedProviderData: ProviderFormValues) => void;
};

export const FormFields: FC<FormFieldsProps> = ({
  providerData,
  fields,
  onChange,
}) => {
  return Object.entries(fields).map(([key, value], idx) => {
    if (key === "endpoint_style") {
      const availableOptions = ["openai", "hf"];
      const displayValues = ["OpenAI", "HuggingFace"];
      return (
        <Flex key={`${key}_${idx}`} direction="column">
          {toPascalCase(key)}
          <Select.Root
            defaultValue={value.toString()}
            onValueChange={(newValue) =>
              onChange({ ...providerData, endpoint_style: newValue })
            }
            disabled={providerData.readonly}
          >
            <Select.Trigger />
            <Select.Content position="popper">
              {availableOptions.map((option, idx) => (
                <Select.Item key={option} value={option}>
                  {displayValues[idx]}
                </Select.Item>
              ))}
            </Select.Content>
          </Select.Root>
        </Flex>
      );
    }

    return (
      <Flex key={`${key}_${idx}`} direction="column" gap="1">
        <label htmlFor={key}>{toPascalCase(key)}</label>
        <TextField.Root
          id={key}
          value={value.toString()}
          onChange={(event) =>
            onChange({ ...providerData, [key]: event.target.value })
          }
          className={classNames({
            [styles.disabledField]: providerData.readonly,
          })}
          disabled={providerData.readonly}
        />
      </Flex>
    );
  });
};

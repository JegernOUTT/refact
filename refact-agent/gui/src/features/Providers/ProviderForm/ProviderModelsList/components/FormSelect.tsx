import { type ReactNode } from "react";

import { FieldSelect, FieldStack } from "../../../../../components/ui";

type FormSelectProps<OptionType> = {
  label: string;
  options?: OptionType[];
  optionTransformer?: (option: OptionType) => OptionType;
  value: string;
  placeholder?: string;
  description?: string;
  isDisabled?: boolean;
  onValueChange?: (value: string) => void;
  children?: ReactNode;
};

export type OptionType = string | null;

export function FormSelect({
  label,
  options,
  value,
  placeholder,
  description,
  isDisabled,
  onValueChange,
  optionTransformer,
}: FormSelectProps<OptionType>) {
  return (
    <FieldStack
      label={label}
      helper={description}
      control={
        <FieldSelect
          value={value}
          placeholder={placeholder}
          disabled={isDisabled}
          onChange={(nextValue) => onValueChange?.(nextValue)}
          options={
            options?.map((option) => {
              if (option !== null) {
                const transformed = optionTransformer
                  ? optionTransformer(option)
                  : option;
                return { value: option, label: transformed };
              }
              return { value: "null", label: "None" };
            }) ?? []
          }
        />
      }
    />
  );
}

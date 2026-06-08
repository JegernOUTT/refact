import { type FC, type ReactNode } from "react";
import { Markdown } from "../../../../../components/Markdown";
import { FieldStack, FieldText } from "../../../../../components/ui";

type FormFieldProps = {
  label: string;
  value?: string;
  placeholder?: string;
  description?: string;
  type?: React.HTMLInputTypeAttribute;
  isDisabled?: boolean;
  max?: string;
  onChange?: React.ChangeEventHandler<HTMLInputElement>;
  children?: ReactNode;
};

export const FormField: FC<FormFieldProps> = ({
  label,
  value,
  placeholder,
  description,
  isDisabled,
  type,
  max,
  onChange,
  children,
}) => {
  return (
    <FieldStack
      label={label}
      helper={description ? <Markdown>{description}</Markdown> : undefined}
      control={
        children ?? (
          <FieldText
            value={value ?? ""}
            placeholder={placeholder}
            type={type}
            max={max}
            onChange={(nextValue) =>
              onChange?.({
                target: { value: nextValue },
                currentTarget: { value: nextValue },
              } as React.ChangeEvent<HTMLInputElement>)
            }
            disabled={isDisabled}
          />
        )
      }
    />
  );
};

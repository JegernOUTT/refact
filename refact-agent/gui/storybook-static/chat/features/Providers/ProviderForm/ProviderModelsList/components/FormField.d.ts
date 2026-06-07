import { TextField } from "@radix-ui/themes";
import { FC, ReactNode } from "react";
type FormFieldProps = {
    label: string;
    value?: string;
    placeholder?: string;
    description?: string;
    type?: TextField.RootProps["type"];
    isDisabled?: boolean;
    max?: string;
    onChange?: React.ChangeEventHandler<HTMLInputElement>;
    children?: ReactNode;
};
/**
 * Reusable form field component with consistent styling
 */
export declare const FormField: FC<FormFieldProps>;
export {};

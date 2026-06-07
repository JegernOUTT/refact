import { JSX } from 'react/jsx-runtime';
import { ReactNode } from "react";
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
/**
 * Type for the options of the form select component
 */
export type OptionType = string | null;
/**
 * Reusable form select component with consistent styling
 */
export declare function FormSelect({ label, options, value, placeholder, description, isDisabled, onValueChange, optionTransformer, }: FormSelectProps<OptionType>): JSX.Element;
export {};

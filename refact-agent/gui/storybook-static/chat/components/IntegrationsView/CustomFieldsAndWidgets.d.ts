import { JSX } from 'react/jsx-runtime';
import { TextField } from "@radix-ui/themes";
export declare const CustomInputField: ({ value, placeholder, type, id, name, size, color, onChange, wasInteracted, }: {
    id?: string;
    wasInteracted?: boolean;
    type?: "number" | "search" | "time" | "text" | "hidden" | "tel" | "url" | "email" | "date" | "password" | "datetime-local" | "month" | "week";
    value?: string;
    name?: string;
    placeholder?: string;
    size?: string;
    width?: string;
    color?: TextField.RootProps["color"];
    onChange?: (value: string) => void;
}) => JSX.Element;
export declare const CustomLabel: ({ label, htmlFor, mt, }: {
    label: string;
    htmlFor?: string;
    mt?: string;
}) => JSX.Element;
export declare const CustomDescriptionField: ({ children, mb, }: {
    children?: string;
    mb?: string;
}) => JSX.Element;
export declare const CustomBoolField: ({ id, name, value, onChange, }: {
    id: string;
    name: string;
    value: boolean;
    onChange: (value: boolean) => void;
}) => JSX.Element;

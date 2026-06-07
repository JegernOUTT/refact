import React from "react";
import { CheckboxProps as RadixCheckboxProps } from "@radix-ui/themes";
export type CheckboxProps = RadixCheckboxProps & {
    children: React.ReactNode;
};
export declare const Checkbox: React.FC<CheckboxProps>;

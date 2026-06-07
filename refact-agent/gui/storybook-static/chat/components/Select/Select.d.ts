import React, { ReactNode } from "react";
import { Select as RadixSelect } from "@radix-ui/themes";
type SeparatorOption = {
    type: "separator";
    key?: string;
};
export type SelectProps = React.ComponentProps<typeof RadixSelect.Root> & {
    onChange: (value: string) => void;
    options: (string | ItemProps | SeparatorOption)[];
    title?: string;
    contentPosition?: "item-aligned" | "popper";
    value?: string;
    disabled?: boolean;
    open?: SelectRootProps["open"];
    defaultOpen?: SelectRootProps["defaultOpen"];
};
export type SelectRootProps = React.ComponentProps<typeof RadixSelect.Root>;
export declare const Root: React.FC<SelectRootProps>;
export type TriggerProps = React.ComponentProps<typeof RadixSelect.Trigger>;
export declare const Trigger: React.FC<TriggerProps>;
export type ContentProps = React.ComponentProps<typeof RadixSelect.Content>;
export declare const Content: React.FC<ContentProps & {
    className?: string;
}>;
export type ItemProps = React.ComponentProps<typeof RadixSelect.Item> & {
    tooltip?: ReactNode;
};
export declare const Item: React.FC<ItemProps & {
    className?: string;
}>;
export type SeparatorProps = React.ComponentProps<typeof RadixSelect.Separator>;
export declare const Separator: React.FC<SeparatorProps>;
export declare const Select: React.FC<SelectProps>;
export {};

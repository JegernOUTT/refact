import React from "react";
import * as RadixCollapsible from "@radix-ui/react-collapsible";
export type CollapsibleProps = Pick<RadixCollapsible.CollapsibleProps, "disabled" | "className" | "defaultOpen"> & React.PropsWithChildren<{
    className?: string;
    disabled?: boolean;
    title?: React.ReactNode;
}>;
export declare const Collapsible: React.FC<CollapsibleProps>;

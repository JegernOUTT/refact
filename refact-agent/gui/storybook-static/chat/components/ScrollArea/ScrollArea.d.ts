import { ScrollArea as RadixScrollArea } from "@radix-ui/themes";
import React from "react";
export type ScrollAreaProps = React.ComponentProps<typeof RadixScrollArea> & {
    className?: string;
    scrollbars?: "vertical" | "horizontal" | "both" | undefined;
    fullHeight?: boolean;
};
export declare const ScrollArea: React.ForwardRefExoticComponent<Omit<ScrollAreaProps, "ref"> & React.RefAttributes<HTMLDivElement>>;

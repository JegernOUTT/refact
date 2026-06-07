import React from "react";
import { type ComboboxStore } from "@ariakit/react";
import { type AnchorRect } from "./utils";
export declare const Popover: React.FC<React.PropsWithChildren & {
    store: ComboboxStore;
    hidden: boolean;
    getAnchorRect: (anchor: HTMLElement | null) => AnchorRect | null;
    maxWidth?: number | null;
}>;

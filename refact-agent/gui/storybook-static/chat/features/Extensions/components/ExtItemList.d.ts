import React from "react";
import type { SkillRegistryItem, CommandRegistryItem } from "../../../services/refact/extensions";
export type RegistryItem = SkillRegistryItem | CommandRegistryItem;
type ExtItemListProps = {
    items: RegistryItem[];
    selectedId: string | null;
    onSelect: (name: string) => void;
    onCreate: () => void;
    onDelete: (name: string, scope: "global" | "local" | "plugin") => void;
};
export declare const ExtItemList: React.FC<ExtItemListProps>;
export {};

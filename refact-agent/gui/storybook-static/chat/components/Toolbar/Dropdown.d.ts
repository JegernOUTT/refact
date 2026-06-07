import React from "react";
export type DropdownNavigationOptions = "stats" | "settings" | "hot keys" | "integrations" | "providers" | "knowledge graph" | "customization" | "default models" | "extensions" | "";
type DropdownProps = {
    handleNavigation: (to: DropdownNavigationOptions) => void;
    triggerClassName?: string;
    useGhostTrigger?: boolean;
};
export declare const Dropdown: React.FC<DropdownProps>;
export {};

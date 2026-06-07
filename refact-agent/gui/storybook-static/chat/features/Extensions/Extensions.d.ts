import React from "react";
import type { Config } from "../Config/configSlice";
export type ExtensionsTab = "skills" | "commands" | "hooks";
export type ExtensionsProps = {
    backFromExtensions: () => void;
    host: Config["host"];
    tabbed: Config["tabbed"];
    initialTab?: ExtensionsTab;
    initialItemId?: string;
    draftId?: string;
};
export declare const Extensions: React.FC<ExtensionsProps>;

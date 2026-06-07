import React from "react";
import { ConfigPatch } from "./configUtils";
type ToolboxCommandFormProps = {
    config: Record<string, unknown>;
    onPatch: (patch: ConfigPatch) => void;
};
export declare const ToolboxCommandForm: React.FC<ToolboxCommandFormProps>;
export {};

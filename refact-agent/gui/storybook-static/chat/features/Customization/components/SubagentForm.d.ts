import React from "react";
import { ConfigPatch } from "./configUtils";
type SubagentFormProps = {
    config: Record<string, unknown>;
    onPatch: (patch: ConfigPatch) => void;
    availableTools?: string[];
};
export declare const SubagentForm: React.FC<SubagentFormProps>;
export {};

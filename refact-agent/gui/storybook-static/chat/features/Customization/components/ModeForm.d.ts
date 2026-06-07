import React from "react";
import { ConfigPatch } from "./configUtils";
type ModeFormProps = {
    config: Record<string, unknown>;
    onPatch: (patch: ConfigPatch) => void;
    availableTools?: string[];
};
export declare const ModeForm: React.FC<ModeFormProps>;
export {};

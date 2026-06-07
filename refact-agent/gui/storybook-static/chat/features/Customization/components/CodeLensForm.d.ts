import React from "react";
import { ConfigPatch } from "./configUtils";
type CodeLensFormProps = {
    config: Record<string, unknown>;
    onPatch: (patch: ConfigPatch) => void;
};
export declare const CodeLensForm: React.FC<CodeLensFormProps>;
export {};

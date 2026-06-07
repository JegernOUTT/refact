import React from "react";
export type ToolParameter = {
    name: string;
    type: string;
    description: string;
    default?: unknown;
};
type ToolParametersEditorProps = {
    parameters: ToolParameter[];
    required: string[];
    onParametersChange: (value: ToolParameter[]) => void;
    onRequiredChange: (value: string[]) => void;
    label?: string;
};
export declare const ToolParametersEditor: React.FC<ToolParametersEditorProps>;
export {};

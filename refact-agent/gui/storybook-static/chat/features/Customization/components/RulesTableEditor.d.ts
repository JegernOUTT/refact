import React from "react";
export type ToolConfirmRule = {
    match: string;
    action: string;
};
type RulesTableEditorProps = {
    value: ToolConfirmRule[];
    onChange: (value: ToolConfirmRule[]) => void;
    label?: string;
};
export declare const RulesTableEditor: React.FC<RulesTableEditorProps>;
export {};

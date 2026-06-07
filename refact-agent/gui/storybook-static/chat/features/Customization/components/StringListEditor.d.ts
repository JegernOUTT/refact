import React from "react";
type StringListEditorProps = {
    value: string[];
    onChange: (value: string[]) => void;
    label?: string;
    placeholder?: string;
    suggestions?: string[];
};
export declare const StringListEditor: React.FC<StringListEditorProps>;
export {};

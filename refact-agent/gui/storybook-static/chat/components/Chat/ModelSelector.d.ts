import React from "react";
export type ModelSelectorProps = {
    disabled?: boolean;
    value: string | undefined;
    onValueChange: (model: string) => void;
    label?: string;
    showLabel?: boolean;
    compact?: boolean;
    defaultValue?: string;
    allowUnset?: boolean;
    unsetLabel?: string;
};
export declare const ModelSelector: React.FC<ModelSelectorProps>;

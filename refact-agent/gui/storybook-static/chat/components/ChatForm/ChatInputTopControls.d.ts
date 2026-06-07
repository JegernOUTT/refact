import React from "react";
import type { Checkbox as CheckboxType } from "./useCheckBoxes";
import type { useAttachedFiles } from "./useCheckBoxes";
export type ChatInputTopControlsProps = {
    checkboxes: Record<string, CheckboxType>;
    onCheckedChange: (name: string, checked: boolean | string) => void;
    attachedFiles: ReturnType<typeof useAttachedFiles>;
    disabled?: boolean;
};
export declare const ChatInputTopControls: React.FC<ChatInputTopControlsProps>;

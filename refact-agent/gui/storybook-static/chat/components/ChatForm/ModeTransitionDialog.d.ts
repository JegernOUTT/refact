import React from "react";
type ModeTransitionDialogProps = {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    chatId: string;
    currentMode: string;
    targetMode: string;
    targetModeTitle: string;
    targetModeDescription: string;
};
export declare const ModeTransitionDialog: React.FC<ModeTransitionDialogProps>;
export {};

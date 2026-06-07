import React from "react";
import { ChatModeThreadDefaults } from "../../services/refact/chatModes";
type ModeSelectProps = {
    selectedMode: string;
    onModeChange: (modeId: string, threadDefaults?: ChatModeThreadDefaults) => void;
    disabled?: boolean;
    onOpenChange?: (open: boolean) => void;
};
export declare const ModeSelect: React.FC<ModeSelectProps>;
export {};

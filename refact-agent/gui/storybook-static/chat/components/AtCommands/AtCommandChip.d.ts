import React from "react";
import type { ChipDisplayInfo } from "../../utils/atCommands";
type AtCommandChipProps = {
    chip: ChipDisplayInfo;
    onClick?: () => void;
};
export declare const AtCommandChip: React.FC<AtCommandChipProps>;
export {};

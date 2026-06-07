import React from "react";
import { DiffChunk } from "../../../services/refact/types";
export type DiffHeaderAction = {
    label: string;
    icon: React.ReactNode;
    onClick: () => void;
    disabled?: boolean;
};
export declare const DiffBlock: React.FC<{
    diff: DiffChunk;
    fileName?: string;
    displayFileName?: string;
    onOpenFile?: () => void;
    actions?: DiffHeaderAction[];
}>;

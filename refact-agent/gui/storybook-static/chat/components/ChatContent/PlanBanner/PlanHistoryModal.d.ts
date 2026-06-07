import React from "react";
import type { PlanHistoryItem } from "../../../features/Chat/Thread/selectors";
type PlanHistoryModalProps = {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    items: PlanHistoryItem[];
};
export declare const PlanHistoryModal: React.FC<PlanHistoryModalProps>;
export {};

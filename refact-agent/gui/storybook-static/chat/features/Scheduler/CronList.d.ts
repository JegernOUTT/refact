import React from "react";
import type { CronTask } from "../../services/refact/schedulerApi";
type CronListProps = {
    tasks: CronTask[];
    isLoading?: boolean;
    deletingId?: string | null;
    onDelete: (id: string) => void;
};
export declare const CronList: React.FC<CronListProps>;
export {};

import React from "react";
import { type CreateCronRequest } from "../../services/refact/schedulerApi";
type CronCreateFormData = Omit<CreateCronRequest, "chat_id" | "mode">;
type CronCreateFormProps = {
    onSubmit: (request: CronCreateFormData) => Promise<void>;
    isLoading?: boolean;
    error?: unknown;
    taskCount: number;
    maxTasks?: number;
};
export declare const CronCreateForm: React.FC<CronCreateFormProps>;
export {};

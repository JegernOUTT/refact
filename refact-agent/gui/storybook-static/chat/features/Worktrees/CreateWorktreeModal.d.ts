import React from "react";
export type CreateWorktreeValues = {
    branch?: string;
    baseBranch?: string;
};
type CreateWorktreeModalProps = {
    open: boolean;
    defaultBranch: string;
    defaultBaseBranch: string;
    baseBranchOptions: string[];
    isCreating: boolean;
    error?: string | null;
    onOpenChange: (open: boolean) => void;
    onCreate: (values: CreateWorktreeValues) => Promise<void>;
};
export declare const CreateWorktreeModal: React.FC<CreateWorktreeModalProps>;
export {};

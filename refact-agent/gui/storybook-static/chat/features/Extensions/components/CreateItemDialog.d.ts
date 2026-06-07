import React from "react";
type ItemType = "skill" | "command";
type CreateItemDialogProps = {
    type: ItemType;
    open: boolean;
    onOpenChange: (open: boolean) => void;
    onCreated: (name: string) => void;
    hasProjectRoot: boolean;
};
export declare const CreateItemDialog: React.FC<CreateItemDialogProps>;
export {};

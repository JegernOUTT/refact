import React from "react";
type ThreadInfoButtonProps = {
    chatId: string | null;
    disabled?: boolean;
    onOpenChange?: (open: boolean) => void;
};
export declare const ThreadInfoButton: React.FC<ThreadInfoButtonProps>;
export {};

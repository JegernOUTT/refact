import React from "react";
import { Usage } from "../../services/refact";
import { Checkpoint } from "../../features/Checkpoints/types";
type MessageFooterProps = {
    messageId?: string;
    onCopy?: () => void;
    onBranch?: (messageId: string) => void;
    onDelete?: (messageId: string) => void;
    usage?: Usage | null;
    checkpoints?: Checkpoint[] | null;
    messageIndex?: number;
};
export declare const MessageFooter: React.FC<MessageFooterProps>;
export declare const MessageWrapper: React.FC<{
    children: React.ReactNode;
}>;
export {};

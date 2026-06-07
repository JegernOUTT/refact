import React from "react";
import type { UserMessage } from "../../services/refact";
import type { Checkpoint } from "../../features/Checkpoints/types";
export type UserInputProps = {
    children: UserMessage["content"];
    messageIndex: number;
    messageId?: string;
    checkpoints?: Checkpoint[];
    onRetry: (index: number, question: UserMessage["content"]) => void;
    onBranch?: (messageId: string) => void;
    onDelete?: (messageId: string) => void;
};
export declare const UserInput: React.NamedExoticComponent<UserInputProps>;

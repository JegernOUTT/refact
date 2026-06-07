import React from "react";
import { UserMessage } from "../../services/refact";
export type ChatContentProps = {
    onRetry: (index: number, question: UserMessage["content"]) => void;
    onStopStreaming: () => void;
};
export declare const ChatContent: React.FC<ChatContentProps>;

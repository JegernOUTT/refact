import React from "react";
type ReasoningContentProps = {
    reasoningContent: string;
    onCopyClick: (text: string) => void;
    isStreaming?: boolean;
    hasMessageContent?: boolean;
    stateKey?: string;
};
export declare const ReasoningContent: React.FC<ReasoningContentProps>;
export {};

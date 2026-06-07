import React from "react";
export type MessageTemplate = {
    role: string;
    content: string;
};
type MessageListEditorProps = {
    value: MessageTemplate[];
    onChange: (value: MessageTemplate[]) => void;
    label?: string;
};
export declare const MessageListEditor: React.FC<MessageListEditorProps>;
export {};

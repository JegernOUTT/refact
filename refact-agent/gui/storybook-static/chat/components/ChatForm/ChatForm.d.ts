import React from "react";
export type SendPolicy = "immediate" | "after_flow";
export type ChatFormProps = {
    onSubmit: (str: string, sendPolicy?: SendPolicy) => void;
    onClose?: () => void;
    className?: string;
};
export declare const ChatForm: React.FC<ChatFormProps>;

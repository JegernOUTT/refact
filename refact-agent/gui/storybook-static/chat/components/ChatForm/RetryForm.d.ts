import React from "react";
import { UserMessage } from "../../services/refact";
export declare const RetryForm: React.FC<{
    value: UserMessage["content"];
    onSubmit: (value: UserMessage["content"]) => void;
    onClose: () => void;
}>;

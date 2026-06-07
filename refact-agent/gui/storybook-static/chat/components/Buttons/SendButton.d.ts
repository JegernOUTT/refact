import React from "react";
type SendButtonProps = {
    disabled?: boolean;
    isStreaming?: boolean;
    queuedCount?: number;
    onSend: () => void;
    onSendImmediately: () => void;
};
export declare const SendButtonWithDropdown: React.FC<SendButtonProps>;
export default SendButtonWithDropdown;

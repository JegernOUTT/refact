import React from "react";
type UnifiedSendButtonProps = {
    disabled?: boolean;
    isStreaming?: boolean;
    hasText: boolean;
    hasMessages: boolean;
    queuedCount?: number;
    onSend: () => void;
    onSendImmediately: () => void;
    onStop: () => void;
    onResend: () => void;
};
export declare const UnifiedSendButton: React.FC<UnifiedSendButtonProps>;
export default UnifiedSendButton;

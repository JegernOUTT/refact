import React from "react";
import type { LucideIcon } from "lucide-react";
import { Clock, RefreshCcw, Send, Square, Zap } from "lucide-react";
import { ComposerTrigger, KitTooltip } from "../ComposerKit";
import styles from "./UnifiedSendButton.module.css";

type UnifiedSendButtonProps = {
  disabled?: boolean;
  isStreaming?: boolean;
  hasText: boolean;
  hasMessages: boolean;
  onSend: () => void;
  onSendImmediately: () => void;
  onStop: () => void;
  onResend: () => void;
};

const SendShape: React.FC<{
  label: string;
  tooltip: string;
  shortcut?: string;
  icon: LucideIcon;
  variant: "primary" | "stop";
  disabled?: boolean;
  onActivate: () => void;
}> = ({
  label,
  tooltip,
  shortcut,
  icon: IconComponent,
  variant,
  disabled,
  onActivate,
}) => (
  <KitTooltip label={tooltip} state={shortcut}>
    <button
      type="button"
      className={styles.send}
      data-variant={variant}
      aria-label={label}
      disabled={disabled}
      onClick={(event) => {
        event.preventDefault();
        onActivate();
      }}
    >
      <IconComponent aria-hidden="true" />
    </button>
  </KitTooltip>
);

export const UnifiedSendButton: React.FC<UnifiedSendButtonProps> = ({
  disabled,
  isStreaming,
  hasText,
  hasMessages,
  onSend,
  onSendImmediately,
  onStop,
  onResend,
}) => {
  if (isStreaming) {
    return (
      <span className={styles.group}>
        {hasText ? (
          <>
            <KitTooltip label="Queue message" state="after tools complete">
              <ComposerTrigger
                iconOnly
                aria-label="Queue message"
                disabled={disabled}
                icon={Clock}
                onClick={(event) => {
                  event.preventDefault();
                  onSend();
                }}
              />
            </KitTooltip>
            <KitTooltip label="Send immediately" state="next turn">
              <ComposerTrigger
                iconOnly
                aria-label="Send immediately"
                disabled={disabled}
                icon={Zap}
                onClick={(event) => {
                  event.preventDefault();
                  onSendImmediately();
                }}
              />
            </KitTooltip>
          </>
        ) : null}
        <SendShape
          label="Stop generation"
          tooltip="Stop generation"
          icon={Square}
          variant="stop"
          onActivate={onStop}
        />
      </span>
    );
  }

  if (!hasText && hasMessages) {
    return (
      <span className={styles.group}>
        <KitTooltip label="Resend last messages">
          <ComposerTrigger
            iconOnly
            aria-label="Resend last messages"
            disabled={disabled}
            icon={RefreshCcw}
            onClick={(event) => {
              event.preventDefault();
              onResend();
            }}
          />
        </KitTooltip>
      </span>
    );
  }

  return (
    <SendShape
      label="Send message"
      tooltip="Send"
      shortcut="Enter"
      icon={Send}
      variant="primary"
      disabled={disabled}
      onActivate={onSend}
    />
  );
};

export default UnifiedSendButton;

import React from "react";
import { RefreshCw } from "lucide-react";
import { useAppSelector, useChatActions } from "../../hooks";
import {
  selectIsStreaming,
  selectIsWaiting,
  selectMessages,
} from "../../features/Chat";
import { IconButton, Popover } from "../ui";

function useResendMessages() {
  const messages = useAppSelector(selectMessages);
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);
  const { regenerate } = useChatActions();

  const handleResend = React.useCallback(() => {
    void regenerate();
  }, [regenerate]);

  const shouldShow = React.useMemo(() => {
    if (messages.length === 0) return false;
    if (isStreaming) return false;
    if (isWaiting) return false;
    return true;
  }, [messages.length, isStreaming, isWaiting]);

  return { handleResend, shouldShow };
}

export const ResendButton = () => {
  const { handleResend, shouldShow } = useResendMessages();

  if (!shouldShow) return null;

  return (
    <Popover>
      <Popover.Trigger asChild>
        <IconButton
          aria-label="Resend last messages"
          icon={RefreshCw}
          onClick={handleResend}
          size="sm"
          variant="plain"
        />
      </Popover.Trigger>
      <Popover.Content side="top" maxWidth="280px">
        Resend last messages
      </Popover.Content>
    </Popover>
  );
};

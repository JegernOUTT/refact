import React from "react";
import { RefreshCw } from "lucide-react";
import { useAppSelector, useChatActions } from "../../hooks";
import {
  selectIsStreamingById,
  selectIsWaitingById,
  selectMessagesById,
  useThreadId,
} from "../../features/Chat";
import { IconButton, Popover } from "../ui";

function useResendMessages() {
  const chatId = useThreadId();
  const messages = useAppSelector((state) => selectMessagesById(state, chatId));
  const isStreaming = useAppSelector((state) =>
    selectIsStreamingById(state, chatId),
  );
  const isWaiting = useAppSelector((state) =>
    selectIsWaitingById(state, chatId),
  );
  const { regenerate } = useChatActions(chatId);

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

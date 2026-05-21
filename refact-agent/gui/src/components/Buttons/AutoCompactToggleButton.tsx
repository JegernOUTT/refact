import { useCallback } from "react";
import { HoverCard, Text } from "@radix-ui/themes";
import { ArchiveIcon } from "@radix-ui/react-icons";
import iconStyles from "./iconButton.module.css";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectCurrentThreadId,
  selectAutoCompactEnabled,
  setAutoCompactEnabled,
} from "../../features/Chat";
import { updateChatParams } from "../../services/refact/chatCommands";
import { selectLspPort, selectApiKey } from "../../features/Config/configSlice";

type AutoCompactToggleButtonProps = {
  disabled?: boolean;
};

export const AutoCompactToggleButton = ({
  disabled,
}: AutoCompactToggleButtonProps) => {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectCurrentThreadId);
  const isEnabled = useAppSelector(selectAutoCompactEnabled);
  const port = useAppSelector(selectLspPort);
  const apiKey = useAppSelector(selectApiKey);

  const handleClick = useCallback(() => {
    if (!chatId || disabled) return;
    const next = !isEnabled;
    dispatch(setAutoCompactEnabled({ chatId, value: next }));
    if (port) {
      void updateChatParams(
        chatId,
        { auto_compact_enabled: next },
        port,
        apiKey ?? undefined,
      ).catch(() => undefined);
    }
  }, [chatId, isEnabled, disabled, port, apiKey, dispatch]);

  const label = isEnabled
    ? "Auto-compression enabled"
    : "Auto-compression disabled";
  const actionLabel = isEnabled
    ? "Auto-compression ON — click to disable"
    : "Auto-compression OFF — click to enable";

  return (
    <HoverCard.Root>
      <HoverCard.Trigger>
        <button
          type="button"
          className={iconStyles.iconButton}
          onClick={handleClick}
          disabled={disabled}
          aria-label={actionLabel}
          aria-pressed={isEnabled}
          data-testid="auto-compact-toggle"
        >
          <ArchiveIcon
            style={
              isEnabled ? { color: "var(--accent-11)" } : { opacity: 0.45 }
            }
          />
        </button>
      </HoverCard.Trigger>
      <HoverCard.Content size="1" side="top">
        <Text as="p" size="2">
          {label}
        </Text>
        <Text as="p" size="1" color="gray">
          Automatically compacts chat context when it grows too large.
        </Text>
      </HoverCard.Content>
    </HoverCard.Root>
  );
};

AutoCompactToggleButton.displayName = "AutoCompactToggleButton";

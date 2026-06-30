import { useCallback } from "react";
import { Text } from "@radix-ui/themes";
import { Archive } from "lucide-react";
import { IconButton } from "../ui";
import { HoverCard } from "../LongTailPrimitives";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectAutoCompactEnabledById,
  setAutoCompactEnabled,
  useThreadId,
} from "../../features/Chat/Thread";
import { updateChatParams } from "../../services/refact/chatCommands";
import { selectConfig, selectApiKey } from "../../features/Config/configSlice";

type AutoCompactToggleButtonProps = {
  disabled?: boolean;
};

export const AutoCompactToggleButton = ({
  disabled,
}: AutoCompactToggleButtonProps) => {
  const dispatch = useAppDispatch();
  const chatId = useThreadId();
  const isEnabled = useAppSelector((state) =>
    selectAutoCompactEnabledById(state, chatId),
  );
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);

  const handleClick = useCallback(() => {
    if (!chatId || disabled) return;
    const next = !isEnabled;
    dispatch(setAutoCompactEnabled({ chatId, value: next }));
    void updateChatParams(
      chatId,
      { auto_compact_enabled: next },
      config,
      apiKey ?? undefined,
    ).catch(() => undefined);
  }, [chatId, isEnabled, disabled, config, apiKey, dispatch]);

  const label = isEnabled
    ? "Auto-compression enabled"
    : "Auto-compression disabled";
  const actionLabel = isEnabled
    ? "Auto-compression ON — click to disable"
    : "Auto-compression OFF — click to enable";

  return (
    <HoverCard>
      <HoverCard.Trigger asChild>
        <IconButton
          aria-label={actionLabel}
          aria-pressed={isEnabled}
          data-testid="auto-compact-toggle"
          disabled={disabled}
          icon={Archive}
          onClick={handleClick}
          size="sm"
          variant={isEnabled ? "primary" : "ghost"}
        />
      </HoverCard.Trigger>
      <HoverCard.Content side="top">
        <Text as="p" size="2">
          {label}
        </Text>
        <Text as="p" size="1" color="gray">
          When on: summarizes older messages as the chat grows, and runs an
          emergency compaction if the provider returns a context-length error.
          When off: both behaviors are disabled and you handle compaction
          manually.
        </Text>
      </HoverCard.Content>
    </HoverCard>
  );
};

AutoCompactToggleButton.displayName = "AutoCompactToggleButton";

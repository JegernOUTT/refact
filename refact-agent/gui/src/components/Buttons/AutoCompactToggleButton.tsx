import { useCallback } from "react";
import { Archive } from "lucide-react";
import { ComposerTrigger, KitTooltip } from "../ComposerKit";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useAppSelector } from "../../hooks/useAppSelector";
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

  const actionLabel = isEnabled
    ? "Auto-compression ON — click to disable"
    : "Auto-compression OFF — click to enable";

  return (
    <KitTooltip
      label="Auto-compression"
      state={isEnabled ? "On" : "Off"}
      stateTone={isEnabled ? "accent" : undefined}
      hint="Summarizes older messages as the chat grows and compacts on context-length errors."
    >
      <ComposerTrigger
        iconOnly
        aria-label={actionLabel}
        aria-pressed={isEnabled}
        data-testid="auto-compact-toggle"
        disabled={disabled}
        icon={Archive}
        onClick={handleClick}
      />
    </KitTooltip>
  );
};

AutoCompactToggleButton.displayName = "AutoCompactToggleButton";

import { useCallback } from "react";
import { Layers } from "lucide-react";
import { IconButton, Tooltip } from "../ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectAutoEnrichmentEnabledById,
  selectMemoryEnrichmentUserTouchedById,
  setAutoEnrichmentEnabled,
  markMemoryEnrichmentUserTouched,
  useThreadId,
} from "../../features/Chat/Thread";
import { updateChatParams } from "../../services/refact/chatCommands";
import { selectConfig, selectApiKey } from "../../features/Config/configSlice";

type AutoEnrichmentToggleButtonProps = {
  disabled?: boolean;
};

export const AutoEnrichmentToggleButton = ({
  disabled,
}: AutoEnrichmentToggleButtonProps) => {
  const dispatch = useAppDispatch();
  const chatId = useThreadId();
  const isEnabled = useAppSelector((state) =>
    selectAutoEnrichmentEnabledById(state, chatId),
  );
  const userTouched = useAppSelector((state) =>
    selectMemoryEnrichmentUserTouchedById(state, chatId),
  );
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);

  const handleClick = useCallback(() => {
    if (!chatId || disabled) return;
    const next = !isEnabled;
    if (!userTouched) {
      dispatch(markMemoryEnrichmentUserTouched({ chatId }));
    }
    dispatch(setAutoEnrichmentEnabled({ chatId, value: next }));
    void updateChatParams(
      chatId,
      { auto_enrichment_enabled: next },
      config,
      apiKey ?? undefined,
    ).catch(() => undefined);
  }, [chatId, isEnabled, userTouched, disabled, config, apiKey, dispatch]);

  const label = isEnabled
    ? "Auto-enrichment ON — click to disable"
    : "Auto-enrichment OFF — click to enable";

  return (
    <Tooltip>
      <Tooltip.Trigger asChild>
        <IconButton
          aria-label={label}
          aria-pressed={isEnabled}
          data-testid="auto-enrichment-toggle"
          disabled={disabled}
          icon={Layers}
          onClick={handleClick}
          size="sm"
          variant={isEnabled ? "primary" : "ghost"}
        />
      </Tooltip.Trigger>
      <Tooltip.Content side="top">{label}</Tooltip.Content>
    </Tooltip>
  );
};

AutoEnrichmentToggleButton.displayName = "AutoEnrichmentToggleButton";

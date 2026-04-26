import { useCallback } from "react";
import { HoverCard, Text } from "@radix-ui/themes";
import { LayersIcon } from "@radix-ui/react-icons";
import iconStyles from "./iconButton.module.css";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectCurrentThreadId,
  selectAutoEnrichmentEnabled,
  selectMemoryEnrichmentUserTouched,
  setAutoEnrichmentEnabled,
  markMemoryEnrichmentUserTouched,
} from "../../features/Chat";
import { updateChatParams } from "../../services/refact/chatCommands";
import { selectLspPort, selectApiKey } from "../../features/Config/configSlice";

type AutoEnrichmentToggleButtonProps = {
  disabled?: boolean;
};

export const AutoEnrichmentToggleButton = ({
  disabled,
}: AutoEnrichmentToggleButtonProps) => {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectCurrentThreadId);
  const isEnabled = useAppSelector(selectAutoEnrichmentEnabled);
  const userTouched = useAppSelector(selectMemoryEnrichmentUserTouched);
  const port = useAppSelector(selectLspPort);
  const apiKey = useAppSelector(selectApiKey);

  const handleClick = useCallback(() => {
    if (!chatId || disabled) return;
    const next = !isEnabled;
    if (!userTouched) {
      dispatch(markMemoryEnrichmentUserTouched({ chatId }));
    }
    dispatch(setAutoEnrichmentEnabled({ chatId, value: next }));
    if (port) {
      void updateChatParams(
        chatId,
        { auto_enrichment_enabled: next },
        port,
        apiKey ?? undefined,
      ).catch(() => undefined);
    }
  }, [chatId, isEnabled, userTouched, disabled, port, apiKey, dispatch]);

  const label = isEnabled
    ? "Auto-enrichment ON — click to disable"
    : "Auto-enrichment OFF — click to enable";

  return (
    <HoverCard.Root>
      <HoverCard.Trigger>
        <button
          type="button"
          className={iconStyles.iconButton}
          onClick={handleClick}
          disabled={disabled}
          aria-label={label}
          aria-pressed={isEnabled}
          data-testid="auto-enrichment-toggle"
        >
          <LayersIcon
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
      </HoverCard.Content>
    </HoverCard.Root>
  );
};

AutoEnrichmentToggleButton.displayName = "AutoEnrichmentToggleButton";

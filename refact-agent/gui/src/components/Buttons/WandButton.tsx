import { useCallback } from "react";
import { HoverCard, Text } from "@radix-ui/themes";
import { WandSparkles } from "lucide-react";
import { IconButton } from "../ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectCurrentThreadId,
  selectManualPreviewItems,
  setManualPreviewItems,
  clearManualPreviewItems,
} from "../../features/Chat";
import { usePreviewMemoryEnrichmentMutation } from "../../services/refact/memoryEnrichment";
import { selectLspPort } from "../../features/Config/configSlice";

type WandButtonProps = {
  currentText: string;
  disabled?: boolean;
  onUpdateText?: (text: string) => void;
};

export const WandButton = ({
  currentText,
  disabled = false,
  onUpdateText,
}: WandButtonProps) => {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectCurrentThreadId);
  const port = useAppSelector(selectLspPort);
  const previewItems = useAppSelector(selectManualPreviewItems);
  const [previewEnrichment, { isLoading }] =
    usePreviewMemoryEnrichmentMutation();

  const hasItems = previewItems.length > 0;

  const handleClick = useCallback(() => {
    if (!chatId || !port || disabled || isLoading) return;
    const text = currentText.trim();
    if (!text) return;
    void previewEnrichment({ chatId, text, port })
      .unwrap()
      .then((result) => {
        if (result.items.length === 0) {
          dispatch(clearManualPreviewItems({ chatId }));
        } else {
          dispatch(setManualPreviewItems({ chatId, items: result.items }));
        }
        if (result.rewrittenText && onUpdateText) {
          onUpdateText(result.rewrittenText);
        }
      })
      .catch(() => undefined);
  }, [
    chatId,
    port,
    currentText,
    disabled,
    isLoading,
    previewEnrichment,
    dispatch,
    onUpdateText,
  ]);

  const label = hasItems
    ? "Re-run context preview"
    : "Preview related memories & context";

  return (
    <HoverCard.Root>
      <HoverCard.Trigger>
        <IconButton
          aria-label={label}
          data-testid="wand-button"
          disabled={disabled || isLoading || !currentText.trim()}
          icon={WandSparkles}
          onClick={handleClick}
          size="sm"
          variant={hasItems ? "primary" : "ghost"}
        />
      </HoverCard.Trigger>
      <HoverCard.Content size="1" side="top">
        <Text as="p" size="2">
          {label}
        </Text>
      </HoverCard.Content>
    </HoverCard.Root>
  );
};

WandButton.displayName = "WandButton";

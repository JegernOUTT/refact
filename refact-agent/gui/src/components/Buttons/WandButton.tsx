import { useCallback } from "react";
import { WandSparkles } from "lucide-react";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useAppSelector } from "../../hooks/useAppSelector";
import {
  selectManualPreviewItemsById,
  setManualPreviewItems,
  clearManualPreviewItems,
  useThreadId,
} from "../../features/Chat/Thread";
import { usePreviewMemoryEnrichmentMutation } from "../../services/refact/memoryEnrichment";
import { selectLspPort } from "../../features/Config/configSlice";
import { ComposerTrigger, KitTooltip } from "../ComposerKit";

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
  const chatId = useThreadId();
  const port = useAppSelector(selectLspPort);
  const previewItems = useAppSelector((state) =>
    selectManualPreviewItemsById(state, chatId),
  );
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
    <KitTooltip
      label="Context preview"
      state={hasItems ? "Ready" : undefined}
      stateTone={"accent"}
    >
      <ComposerTrigger
        iconOnly
        aria-label={label}
        aria-pressed={hasItems}
        data-testid="wand-button"
        disabled={disabled || isLoading || !currentText.trim()}
        icon={WandSparkles}
        onClick={handleClick}
      />
    </KitTooltip>
  );
};

WandButton.displayName = "WandButton";

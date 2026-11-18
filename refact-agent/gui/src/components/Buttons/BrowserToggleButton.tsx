import { forwardRef } from "react";
import { Globe } from "lucide-react";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useAppSelector } from "../../hooks/useAppSelector";
import {
  BROWSER_SURFACE_KEY,
  openTab,
  closeTab,
  selectBrowserSurfaceOpen,
} from "../../features/Workspace";
import { ComposerTrigger, KitTooltip } from "../ComposerKit";

type BrowserToggleButtonProps = {
  chatId: string;
  disabled?: boolean;
};

export const BrowserToggleButton = forwardRef<
  HTMLButtonElement,
  BrowserToggleButtonProps
>(({ disabled }, ref) => {
  const dispatch = useAppDispatch();
  const isOpen = useAppSelector(selectBrowserSurfaceOpen);
  const label = isOpen ? "Close browser" : "Open browser";

  return (
    <KitTooltip
      label={label}
      state={isOpen ? "On" : undefined}
      stateTone="accent"
    >
      <ComposerTrigger
        iconOnly
        aria-label={label}
        aria-pressed={isOpen}
        disabled={disabled}
        icon={Globe}
        onClick={() =>
          dispatch((isOpen ? closeTab : openTab)(BROWSER_SURFACE_KEY))
        }
        ref={ref}
      />
    </KitTooltip>
  );
});

BrowserToggleButton.displayName = "BrowserToggleButton";

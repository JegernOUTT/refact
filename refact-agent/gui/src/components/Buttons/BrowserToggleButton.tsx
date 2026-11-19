import { forwardRef, useId } from "react";
import { Globe } from "lucide-react";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useAppSelector } from "../../hooks/useAppSelector";
import { hideBrowserTabsForChat } from "../../features/Workspace";
import {
  selectBrowserShowInProgress,
  selectChatBrowserViewTabIds,
  selectChatHasUsableBrowserView,
} from "../../features/Browser/browserViews";
import { useProjectBrowserTabActions } from "../../features/Browser/useProjectBrowserTabActions";
import { browserApi } from "../../services/refact/browser";
import { ComposerTrigger, KitTooltip } from "../ComposerKit";
import styles from "./BrowserToggleButton.module.css";

type BrowserToggleButtonProps = {
  chatId: string;
  disabled?: boolean;
};

export const BrowserToggleButton = forwardRef<
  HTMLButtonElement,
  BrowserToggleButtonProps
>(({ chatId, disabled }, ref) => {
  const dispatch = useAppDispatch();
  const errorId = useId();
  const hasViews = useAppSelector(
    (state) => selectChatBrowserViewTabIds(state, chatId).length > 0,
  );
  browserApi.useProjectBrowserStatusQuery(undefined, { skip: !hasViews });
  const shown = useAppSelector((state) =>
    selectChatHasUsableBrowserView(state, chatId),
  );
  const showing = useAppSelector((state) =>
    selectBrowserShowInProgress(state, chatId),
  );
  const actions = useProjectBrowserTabActions();
  const label = shown ? "Hide browser" : "Show browser";

  return (
    <>
      <KitTooltip
        label={actions.error ?? label}
        state={shown ? "On" : undefined}
        stateTone="accent"
      >
        <ComposerTrigger
          iconOnly
          aria-label={label}
          aria-pressed={shown}
          aria-describedby={actions.error ? errorId : undefined}
          disabled={Boolean(disabled) || actions.pending || showing}
          icon={Globe}
          onClick={() => {
            if (shown) dispatch(hideBrowserTabsForChat(chatId));
            else void actions.showBrowserForChat(chatId);
          }}
          ref={ref}
        />
      </KitTooltip>
      {actions.error && (
        <span id={errorId} role="alert" className={styles.srOnly}>
          {actions.error}
        </span>
      )}
    </>
  );
});

BrowserToggleButton.displayName = "BrowserToggleButton";

import { forwardRef, useCallback, useRef, useState } from "react";
import { HoverCard, Text } from "@radix-ui/themes";
import { GlobeIcon } from "@radix-ui/react-icons";
import iconStyles from "./iconButton.module.css";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectBrowserUiOpen,
  selectBrowserRuntime,
  openBrowserUi,
  closeBrowserUi,
  setBrowserRuntime,
  updateBrowserFrame,
  makeBrowserRuntime,
} from "../../features/Browser";
import { browserApi } from "../../services/refact/browser";

type BrowserToggleButtonProps = {
  chatId: string;
  disabled?: boolean;
};

export const BrowserToggleButton = forwardRef<
  HTMLButtonElement,
  BrowserToggleButtonProps
>(({ chatId, disabled }, ref) => {
  const dispatch = useAppDispatch();
  const isOpen = useAppSelector((state) => selectBrowserUiOpen(state, chatId));
  const runtime = useAppSelector((state) =>
    selectBrowserRuntime(state, chatId),
  );
  const [busy, setBusy] = useState(false);
  const [browserStart] = browserApi.useBrowserStartMutation();
  const [browserStop] = browserApi.useBrowserStopMutation();
  const [browserScreenshot] = browserApi.useBrowserScreenshotMutation();
  const requestIdRef = useRef(0);
  // Stable ref to the current runtime_id so the async start callback can check without
  // adding `runtime` to handleClick's deps (which would recreate it on every SSE frame).
  const runtimeIdRef = useRef<string | undefined>(runtime?.runtime_id);
  runtimeIdRef.current = runtime?.runtime_id;

  const handleClick = useCallback(() => {
    if (busy) return;

    if (isOpen) {
      // Invalidate any in-flight start so its dispatches are ignored
      const requestId = ++requestIdRef.current;
      setBusy(true);
      void (async () => {
        try {
          await browserStop({ chat_id: chatId }).unwrap();
        } catch {
          // stop failed — close UI anyway since user explicitly requested it
        } finally {
          if (requestIdRef.current === requestId) {
            dispatch(closeBrowserUi({ chatId }));
            setBusy(false);
          }
        }
      })();
      return;
    }

    const requestId = ++requestIdRef.current;
    dispatch(openBrowserUi({ chatId }));
    setBusy(true);
    void (async () => {
      try {
        const result = await browserStart({ chat_id: chatId }).unwrap();
        if (requestIdRef.current !== requestId) return;
        // Only reset runtime if this is a new session or runtime_id changed; preserve
        // existing timeline/flags set by SSE if we're reconnecting to the same session.
        if (result.status !== "already_running" || runtimeIdRef.current !== result.runtime_id) {
          dispatch(
            setBrowserRuntime({
              chatId,
              runtime: makeBrowserRuntime(result.runtime_id),
            }),
          );
        }
        const screenshotResult = await browserScreenshot({
          chat_id: chatId,
          full_page: false,
        }).unwrap();
        if (requestIdRef.current !== requestId) return;
        dispatch(
          updateBrowserFrame({
            chatId,
            frame: {
              mime: screenshotResult.mime,
              data: screenshotResult.data,
              diff_boxes: [],
            },
          }),
        );
      } catch {
        if (requestIdRef.current === requestId) {
          dispatch(closeBrowserUi({ chatId }));
        }
      } finally {
        if (requestIdRef.current === requestId) {
          setBusy(false);
        }
      }
    })();
  }, [
    isOpen,
    busy,
    chatId,
    dispatch,
    browserStart,
    browserStop,
    browserScreenshot,
  ]);

  const isActive = isOpen && runtime?.connected;
  const label = busy
    ? isOpen
      ? "Stopping browser…"
      : "Starting browser…"
    : isOpen
      ? "Stop browser"
      : "Open browser";

  return (
    <HoverCard.Root>
      <HoverCard.Trigger>
        <button
          type="button"
          className={iconStyles.iconButton}
          aria-label={label}
          disabled={busy || disabled}
          onClick={handleClick}
          ref={ref}
        >
          <GlobeIcon
            style={isActive ? { color: "var(--green-11)" } : undefined}
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
});

BrowserToggleButton.displayName = "BrowserToggleButton";

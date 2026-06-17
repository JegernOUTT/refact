import { forwardRef, useCallback, useRef, useState } from "react";
import { Globe } from "lucide-react";
import { IconButton, Tooltip } from "../ui";
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
  const runtimeIdRef = useRef<string | undefined>(runtime?.runtime_id);
  runtimeIdRef.current = runtime?.runtime_id;

  const handleClick = useCallback(() => {
    if (busy) return;

    if (isOpen) {
      const requestId = ++requestIdRef.current;
      setBusy(true);
      void (async () => {
        try {
          await browserStop({ chat_id: chatId }).unwrap();
        } catch {
          dispatch(closeBrowserUi({ chatId }));
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
        if (
          result.status !== "already_running" ||
          runtimeIdRef.current !== result.runtime_id
        ) {
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
    <Tooltip>
      <Tooltip.Trigger asChild>
        <IconButton
          aria-label={label}
          disabled={busy || disabled}
          icon={Globe}
          onClick={handleClick}
          ref={ref}
          size="sm"
          variant={isActive ? "primary" : "ghost"}
        />
      </Tooltip.Trigger>
      <Tooltip.Content side="top">{label}</Tooltip.Content>
    </Tooltip>
  );
});

BrowserToggleButton.displayName = "BrowserToggleButton";

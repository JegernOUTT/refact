import { useCallback, useState } from "react";
import classNames from "classnames";
import { Tooltip } from "@radix-ui/themes";
import {
  PlayIcon,
  StopIcon,
  CameraIcon,
  ImageIcon,
  ViewGridIcon,
} from "@radix-ui/react-icons";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { browserApi } from "../../services/refact/browser";
import {
  selectBrowserRuntime,
  toggleAttachScreenshotOnSend,
  setBrowserRuntime,
  removeBrowserRuntime,
  closeBrowserUi,
  makeBrowserRuntime,
} from "./browserSlice";
import { addThreadImage } from "../Chat/Thread/actions";
import styles from "./Browser.module.css";

type BrowserToolbarProps = {
  chatId: string;
};

interface LoadingFlags {
  start: boolean;
  stop: boolean;
  screenshot: boolean;
  fullpage: boolean;
}

const defaultLoading: LoadingFlags = {
  start: false,
  stop: false,
  screenshot: false,
  fullpage: false,
};

export const BrowserToolbar = ({ chatId }: BrowserToolbarProps) => {
  const dispatch = useAppDispatch();
  const runtime = useAppSelector((state) =>
    selectBrowserRuntime(state, chatId),
  );
  const [loading, setLoading] = useState<LoadingFlags>({
    ...defaultLoading,
  });

  const [browserStart] = browserApi.useBrowserStartMutation();
  const [browserStop] = browserApi.useBrowserStopMutation();
  const [browserScreenshot] = browserApi.useBrowserScreenshotMutation();

  const withLoading = useCallback(
    async (key: keyof LoadingFlags, fn: () => Promise<void>) => {
      setLoading((prev) => ({ ...prev, [key]: true }));
      try {
        await fn();
      } finally {
        setLoading((prev) => ({ ...prev, [key]: false }));
      }
    },
    [],
  );

  const handleStart = useCallback(() => {
    void withLoading("start", async () => {
      const result = await browserStart({ chat_id: chatId }).unwrap();
      // Only reset runtime state if this is a genuinely new session or the runtime_id changed.
      // If already_running with the same id, preserve existing timeline/flags set by SSE.
      if (
        result.status !== "already_running" ||
        runtime?.runtime_id !== result.runtime_id
      ) {
        dispatch(
          setBrowserRuntime({
            chatId,
            runtime: makeBrowserRuntime(result.runtime_id),
          }),
        );
      }
    });
  }, [browserStart, chatId, dispatch, runtime, withLoading]);

  const handleStop = useCallback(() => {
    void withLoading("stop", async () => {
      await browserStop({ chat_id: chatId }).unwrap();
      dispatch(removeBrowserRuntime({ chatId }));
      // Close the panel — requirement: panels disappear when session ends
      dispatch(closeBrowserUi({ chatId }));
    });
  }, [browserStop, chatId, dispatch, withLoading]);

  const handleScreenshot = useCallback(
    (fullPage: boolean) => {
      const key: keyof LoadingFlags = fullPage ? "fullpage" : "screenshot";
      void withLoading(key, async () => {
        const result = await browserScreenshot({
          chat_id: chatId,
          full_page: fullPage,
        }).unwrap();
        const ext = result.mime === "image/png" ? "png" : "jpg";
        dispatch(
          addThreadImage({
            id: chatId,
            image: {
              name: fullPage ? `full_page.${ext}` : `screenshot.${ext}`,
              content: `data:${result.mime};base64,${result.data}`,
              type: result.mime,
            },
          }),
        );
      });
    },
    [browserScreenshot, chatId, dispatch, withLoading],
  );

  const handleToggleScreenshotOnSend = useCallback(() => {
    dispatch(toggleAttachScreenshotOnSend({ chatId }));
  }, [dispatch, chatId]);

  const isConnected = runtime?.connected ?? false;

  return (
    <div className={styles.browserToolbar}>
      {!isConnected ? (
        <Tooltip content="Start browser">
          <button
            type="button"
            className={styles.toolbarIconButton}
            onClick={handleStart}
            disabled={loading.start}
            aria-label="Start browser"
          >
            <PlayIcon />
          </button>
        </Tooltip>
      ) : (
        <Tooltip content="Stop browser">
          <button
            type="button"
            className={classNames(
              styles.toolbarIconButton,
              styles.toolbarIconButtonDanger,
            )}
            onClick={handleStop}
            disabled={loading.stop}
            aria-label="Stop browser"
          >
            <StopIcon />
          </button>
        </Tooltip>
      )}

      <div className={styles.toolbarSeparator} />

      <Tooltip content="Screenshot (viewport)">
        <button
          type="button"
          className={styles.toolbarIconButton}
          onClick={() => handleScreenshot(false)}
          disabled={!isConnected || loading.screenshot}
          aria-label="Screenshot"
        >
          <CameraIcon />
        </button>
      </Tooltip>

      <Tooltip content="Screenshot (full page)">
        <button
          type="button"
          className={styles.toolbarIconButton}
          onClick={() => handleScreenshot(true)}
          disabled={!isConnected || loading.fullpage}
          aria-label="Full page screenshot"
        >
          <ImageIcon />
        </button>
      </Tooltip>

      <div className={styles.toolbarSeparator} />

      <Tooltip
        content={
          runtime?.attach_screenshot_on_send
            ? "Auto-screenshot on send: ON"
            : "Auto-screenshot on send: OFF"
        }
      >
        <button
          type="button"
          className={classNames(styles.toolbarIconButton, {
            [styles.toolbarIconButtonActive]:
              runtime?.attach_screenshot_on_send ?? false,
          })}
          onClick={handleToggleScreenshotOnSend}
          disabled={!isConnected}
          aria-label="Auto-screenshot on send"
        >
          <ViewGridIcon />
        </button>
      </Tooltip>
    </div>
  );
};

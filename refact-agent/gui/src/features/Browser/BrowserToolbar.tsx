import { useCallback, useState } from "react";
import classNames from "classnames";
import { Camera, Columns3, Image, Play, Square } from "lucide-react";
import { IconButton, Tooltip } from "../../components/ui";
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
        <Tooltip>
          <Tooltip.Trigger asChild>
            <IconButton
              className={styles.toolbarIconButton}
              onClick={handleStart}
              disabled={loading.start}
              aria-label="Start browser"
              icon={Play}
              size="sm"
            />
          </Tooltip.Trigger>
          <Tooltip.Content>Start browser</Tooltip.Content>
        </Tooltip>
      ) : (
        <Tooltip>
          <Tooltip.Trigger asChild>
            <IconButton
              className={classNames(
                styles.toolbarIconButton,
                styles.toolbarIconButtonDanger,
              )}
              onClick={handleStop}
              disabled={loading.stop}
              aria-label="Stop browser"
              icon={Square}
              size="sm"
            />
          </Tooltip.Trigger>
          <Tooltip.Content>Stop browser</Tooltip.Content>
        </Tooltip>
      )}

      <div className={styles.toolbarSeparator} />

      <Tooltip>
        <Tooltip.Trigger asChild>
          <IconButton
            className={styles.toolbarIconButton}
            onClick={() => handleScreenshot(false)}
            disabled={!isConnected || loading.screenshot}
            aria-label="Screenshot"
            icon={Camera}
            size="sm"
          />
        </Tooltip.Trigger>
        <Tooltip.Content>Screenshot (viewport)</Tooltip.Content>
      </Tooltip>

      <Tooltip>
        <Tooltip.Trigger asChild>
          <IconButton
            className={styles.toolbarIconButton}
            onClick={() => handleScreenshot(true)}
            disabled={!isConnected || loading.fullpage}
            aria-label="Full page screenshot"
            icon={Image}
            size="sm"
          />
        </Tooltip.Trigger>
        <Tooltip.Content>Screenshot (full page)</Tooltip.Content>
      </Tooltip>

      <div className={styles.toolbarSeparator} />

      <Tooltip>
        <Tooltip.Trigger asChild>
          <IconButton
            className={classNames(styles.toolbarIconButton, {
              [styles.toolbarIconButtonActive]:
                runtime?.attach_screenshot_on_send ?? false,
            })}
            onClick={handleToggleScreenshotOnSend}
            disabled={!isConnected}
            aria-label="Auto-screenshot on send"
            icon={Columns3}
            size="sm"
          />
        </Tooltip.Trigger>
        <Tooltip.Content>
          {runtime?.attach_screenshot_on_send
            ? "Auto-screenshot on send: ON"
            : "Auto-screenshot on send: OFF"}
        </Tooltip.Content>
      </Tooltip>
    </div>
  );
};

import { useCallback, useState } from "react";
import classNames from "classnames";
import { Tooltip } from "@radix-ui/themes";
import {
  PlayIcon,
  StopIcon,
  CameraIcon,
  CursorArrowIcon,
  GlobeIcon,
  CodeIcon,
  ClipboardCopyIcon,
  ImageIcon,
  ViewGridIcon,
  ActivityLogIcon,
  VideoIcon,
  ReaderIcon,
} from "@radix-ui/react-icons";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { browserApi } from "../../services/refact/browser";
import {
  selectBrowserRuntime,
  toggleAttachScreenshotOnSend,
  setPickerActive,
  setBrowserRuntime,
  removeBrowserRuntime,
  closeBrowserUi,
  makeBrowserRuntime,
  updateBrowserFrame,
} from "./browserSlice";
import { sendUserMessage } from "../../services/refact/chatCommands";
import { selectLspPort, selectApiKey } from "../Config/configSlice";
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
  actions: boolean;
  console: boolean;
  network: boolean;
  curl: boolean;
  pick: boolean;
  record: boolean;
  summarize: boolean;
  extract: boolean;
}

const defaultLoading: LoadingFlags = {
  start: false,
  stop: false,
  screenshot: false,
  fullpage: false,
  actions: false,
  console: false,
  network: false,
  curl: false,
  pick: false,
  record: false,
  summarize: false,
  extract: false,
};

export const BrowserToolbar = ({ chatId }: BrowserToolbarProps) => {
  const dispatch = useAppDispatch();
  const runtime = useAppSelector((state) =>
    selectBrowserRuntime(state, chatId),
  );
  const port = useAppSelector(selectLspPort);
  const apiKey = useAppSelector(selectApiKey);
  const [loading, setLoading] = useState<LoadingFlags>({
    ...defaultLoading,
  });

  const [browserStart] = browserApi.useBrowserStartMutation();
  const [browserStop] = browserApi.useBrowserStopMutation();
  const [browserScreenshot] = browserApi.useBrowserScreenshotMutation();
  const [browserContext] = browserApi.useBrowserContextMutation();
  const [browserCurl] = browserApi.useBrowserCurlMutation();
  const [browserElementPick] = browserApi.useBrowserElementPickMutation();
  const [browserElementPickResult] =
    browserApi.useBrowserElementPickResultMutation();
  const [browserRecordAnimation] =
    browserApi.useBrowserRecordAnimationMutation();

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
      if (result.status !== "already_running" || runtime?.runtime_id !== result.runtime_id) {
        dispatch(setBrowserRuntime({ chatId, runtime: makeBrowserRuntime(result.runtime_id) }));
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
        dispatch(
          addThreadImage({
            id: chatId,
            image: {
              name: fullPage ? "full_page.png" : "screenshot.png",
              content: `data:${result.mime};base64,${result.data}`,
              type: result.mime,
            },
          }),
        );
      });
    },
    [browserScreenshot, chatId, dispatch, withLoading],
  );

  const handleContext = useCallback(
    (field: "actions" | "console" | "network", label: keyof LoadingFlags) => {
      void withLoading(label, async () => {
        const result = await browserContext({
          chat_id: chatId,
          skip_cursor: true,
        }).unwrap();
        const content = JSON.stringify(result[field], null, 2);
        if (port) {
          await sendUserMessage(
            chatId,
            content,
            port,
            apiKey ?? undefined,
          );
        }
      });
    },
    [browserContext, chatId, port, apiKey, withLoading],
  );

  const handleCurl = useCallback(() => {
    void withLoading("curl", async () => {
      const result = await browserCurl({ chat_id: chatId }).unwrap();
      if (port) {
        await sendUserMessage(
          chatId,
          result.curl,
          port,
          apiKey ?? undefined,
        );
      }
    });
  }, [browserCurl, chatId, port, apiKey, withLoading]);

  const handleElementPick = useCallback(() => {
    dispatch(setPickerActive({ chatId, active: true }));
    void withLoading("pick", async () => {
      try {
        await browserElementPick({ chat_id: chatId }).unwrap();
        const pollInterval = 500;
        const maxAttempts = 60;
        for (let i = 0; i < maxAttempts; i++) {
          await new Promise((r) => setTimeout(r, pollInterval));
          const pickResult = await browserElementPickResult({
            chat_id: chatId,
          }).unwrap();
          if ("status" in pickResult) {
            continue;
          }
          if ("selector" in pickResult) {
            const text = `Selector: ${pickResult.selector}\nText: ${pickResult.innerText}\nBbox: ${JSON.stringify(pickResult.bbox)}`;
            if (port) {
              await sendUserMessage(chatId, text, port, apiKey ?? undefined);
            }
          }
          break;
        }
      } finally {
        dispatch(setPickerActive({ chatId, active: false }));
      }
    });
  }, [
    browserElementPick,
    browserElementPickResult,
    chatId,
    dispatch,
    port,
    apiKey,
    withLoading,
  ]);

  const handleRecordAnimation = useCallback(() => {
    void withLoading("record", async () => {
      const result = await browserRecordAnimation({
        chat_id: chatId,
      }).unwrap();
      for (const frame of result.frames) {
        dispatch(
          addThreadImage({
            id: chatId,
            image: {
              name: "animation_frame.png",
              content: `data:${frame.mime};base64,${frame.data}`,
              type: frame.mime,
            },
          }),
        );
      }
    });
  }, [browserRecordAnimation, chatId, dispatch, withLoading]);

  const handleSummarizePage = useCallback(() => {
    void withLoading("summarize", async () => {
      const result = await browserScreenshot({
        chat_id: chatId,
        full_page: false,
      }).unwrap();
      dispatch(
        updateBrowserFrame({
          chatId,
          frame: { mime: result.mime, data: result.data, diff_boxes: [] },
        }),
      );
      dispatch(
        addThreadImage({
          id: chatId,
          image: {
            name: "screenshot.png",
            content: `data:${result.mime};base64,${result.data}`,
            type: result.mime,
          },
        }),
      );
      if (port) {
        await sendUserMessage(
          chatId,
          "Summarize this page",
          port,
          apiKey ?? undefined,
        );
      }
    });
  }, [browserScreenshot, chatId, port, apiKey, dispatch, withLoading]);

  const handleExtractJson = useCallback(() => {
    void withLoading("extract", async () => {
      const result = await browserScreenshot({
        chat_id: chatId,
        full_page: false,
      }).unwrap();
      dispatch(
        updateBrowserFrame({
          chatId,
          frame: { mime: result.mime, data: result.data, diff_boxes: [] },
        }),
      );
      dispatch(
        addThreadImage({
          id: chatId,
          image: {
            name: "screenshot.png",
            content: `data:${result.mime};base64,${result.data}`,
            type: result.mime,
          },
        }),
      );
      if (port) {
        await sendUserMessage(
          chatId,
          "Extract data as JSON from tables/lists",
          port,
          apiKey ?? undefined,
        );
      }
    });
  }, [browserScreenshot, chatId, port, apiKey, dispatch, withLoading]);

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
            className={classNames(styles.toolbarIconButton, styles.toolbarIconButtonDanger)}
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

      <Tooltip content="Paste recorded actions into chat">
        <button
          type="button"
          className={styles.toolbarIconButton}
          onClick={() => handleContext("actions", "actions")}
          disabled={!isConnected || loading.actions}
          aria-label="Actions"
        >
          <ActivityLogIcon />
        </button>
      </Tooltip>

      <Tooltip content="Paste console log into chat">
        <button
          type="button"
          className={styles.toolbarIconButton}
          onClick={() => handleContext("console", "console")}
          disabled={!isConnected || loading.console}
          aria-label="Console"
        >
          <CodeIcon />
        </button>
      </Tooltip>

      <Tooltip content="Paste network requests into chat">
        <button
          type="button"
          className={styles.toolbarIconButton}
          onClick={() => handleContext("network", "network")}
          disabled={!isConnected || loading.network}
          aria-label="Network"
        >
          <GlobeIcon />
        </button>
      </Tooltip>

      <Tooltip content="Paste last request as cURL into chat">
        <button
          type="button"
          className={styles.toolbarIconButton}
          onClick={handleCurl}
          disabled={!isConnected || loading.curl}
          aria-label="cURL"
        >
          <ClipboardCopyIcon />
        </button>
      </Tooltip>

      <Tooltip content="Pick element from page">
        <button
          type="button"
          className={classNames(styles.toolbarIconButton, {
            [styles.toolbarIconButtonActive]: runtime?.picker_active ?? false,
          })}
          onClick={handleElementPick}
          disabled={!isConnected || loading.pick}
          aria-label="Pick element"
        >
          <CursorArrowIcon />
        </button>
      </Tooltip>

      <Tooltip content={runtime?.attach_screenshot_on_send ? "Auto-screenshot on send: ON" : "Auto-screenshot on send: OFF"}>
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

      <div className={styles.toolbarSeparator} />

      <Tooltip content="Record animation frames">
        <button
          type="button"
          className={styles.toolbarIconButton}
          onClick={handleRecordAnimation}
          disabled={!isConnected || loading.record}
          aria-label="Record animation"
        >
          <VideoIcon />
        </button>
      </Tooltip>

      <Tooltip content="Summarize page">
        <button
          type="button"
          className={styles.toolbarIconButton}
          onClick={handleSummarizePage}
          disabled={!isConnected || loading.summarize}
          aria-label="Summarize page"
        >
          <ReaderIcon />
        </button>
      </Tooltip>

      <Tooltip content="Extract JSON from page">
        <button
          type="button"
          className={styles.toolbarIconButton}
          onClick={handleExtractJson}
          disabled={!isConnected || loading.extract}
          aria-label="Extract JSON"
        >
          <ViewGridIcon />
        </button>
      </Tooltip>
    </div>
  );
};

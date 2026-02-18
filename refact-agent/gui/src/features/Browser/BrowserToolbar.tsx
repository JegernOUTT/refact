import { useCallback, useState } from "react";
import classNames from "classnames";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { browserApi } from "../../services/refact/browser";
import {
  selectBrowserRuntime,
  toggleAttachScreenshotOnSend,
  setPickerActive,
  setBrowserRuntime,
  removeBrowserRuntime,
  markBrowserDetached,
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
  handoff: boolean;
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
  handoff: false,
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
  const [browserHandoff] = browserApi.useBrowserHandoffMutation();

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
      dispatch(
        setBrowserRuntime({
          chatId,
          runtime: {
            runtime_id: result.runtime_id,
            connected: true,
            active_tab: null,
            url: null,
            title: null,
            tabs: [],
            latest_frame: null,
            picker_active: false,
            attach_screenshot_on_send: false,
            timeline: [],
            timeline_open: false,
            timeline_filter_source: "all",
            timeline_filter_type: null,
            notification: null,
            oversize_info: null,
          },
        }),
      );
    });
  }, [browserStart, chatId, dispatch, withLoading]);

  const handleStop = useCallback(() => {
    void withLoading("stop", async () => {
      await browserStop({ chat_id: chatId }).unwrap();
      dispatch(removeBrowserRuntime({ chatId }));
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
          if ("status" in pickResult && pickResult.status === "waiting") {
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
      await browserScreenshot({
        chat_id: chatId,
        full_page: false,
      }).unwrap();
      if (port) {
        await sendUserMessage(
          chatId,
          "Summarize this page",
          port,
          apiKey ?? undefined,
        );
      }
    });
  }, [browserScreenshot, chatId, port, apiKey, withLoading]);

  const handleExtractJson = useCallback(() => {
    void withLoading("extract", async () => {
      await browserScreenshot({
        chat_id: chatId,
        full_page: false,
      }).unwrap();
      if (port) {
        await sendUserMessage(
          chatId,
          "Extract data as JSON from tables/lists",
          port,
          apiKey ?? undefined,
        );
      }
    });
  }, [browserScreenshot, chatId, port, apiKey, withLoading]);

  const handleHandoff = useCallback(
    (toChatId: string) => {
      void withLoading("handoff", async () => {
        await browserHandoff({
          from_chat_id: chatId,
          to_chat_id: toChatId,
        }).unwrap();
        dispatch(markBrowserDetached({ chatId }));
        dispatch(
          setBrowserRuntime({
            chatId: toChatId,
            runtime: {
              runtime_id: runtime?.runtime_id ?? "",
              connected: true,
              active_tab: null,
              url: runtime?.url ?? null,
              title: runtime?.title ?? null,
              tabs: [],
              latest_frame: runtime?.latest_frame ?? null,
              picker_active: false,
              attach_screenshot_on_send: false,
              timeline: [],
              timeline_open: false,
              timeline_filter_source: "all",
              timeline_filter_type: null,
              notification: {
                type: "attached",
                message: "Browser session attached",
              },
              oversize_info: null,
            },
          }),
        );
      });
    },
    [browserHandoff, chatId, dispatch, runtime, withLoading],
  );

  const handleToggleScreenshotOnSend = useCallback(() => {
    dispatch(toggleAttachScreenshotOnSend({ chatId }));
  }, [dispatch, chatId]);

  const isConnected = runtime?.connected ?? false;

  return (
    <div className={styles.browserToolbar}>
      {!isConnected ? (
        <button
          type="button"
          className={styles.toolbarButton}
          onClick={handleStart}
          disabled={loading.start}
        >
          {loading.start ? "Starting…" : "▶️ Start"}
        </button>
      ) : (
        <button
          type="button"
          className={styles.toolbarButton}
          onClick={handleStop}
          disabled={loading.stop}
        >
          {loading.stop ? "Stopping…" : "⏹️ Stop"}
        </button>
      )}

      <button
        type="button"
        className={styles.toolbarButton}
        onClick={() => {
          const target = window.prompt("Target chat ID for handoff:");
          if (target) handleHandoff(target);
        }}
        disabled={!isConnected || loading.handoff}
      >
        {loading.handoff ? "…" : "🔄"} Handoff
      </button>

      <div className={styles.toolbarSeparator} />

      <button
        type="button"
        className={styles.toolbarButton}
        onClick={() => handleScreenshot(false)}
        disabled={!isConnected || loading.screenshot}
      >
        {loading.screenshot ? "…" : "📷"} Screenshot
      </button>
      <button
        type="button"
        className={styles.toolbarButton}
        onClick={() => handleScreenshot(true)}
        disabled={!isConnected || loading.fullpage}
      >
        {loading.fullpage ? "…" : "📄"} Full Page
      </button>

      <div className={styles.toolbarSeparator} />

      <button
        type="button"
        className={styles.toolbarButton}
        onClick={() => handleContext("actions", "actions")}
        disabled={!isConnected || loading.actions}
      >
        {loading.actions ? "…" : "📋"} Actions
      </button>
      <button
        type="button"
        className={styles.toolbarButton}
        onClick={() => handleContext("console", "console")}
        disabled={!isConnected || loading.console}
      >
        {loading.console ? "…" : "⚠️"} Console
      </button>
      <button
        type="button"
        className={styles.toolbarButton}
        onClick={() => handleContext("network", "network")}
        disabled={!isConnected || loading.network}
      >
        {loading.network ? "…" : "🌐"} Network
      </button>
      <button
        type="button"
        className={styles.toolbarButton}
        onClick={handleCurl}
        disabled={!isConnected || loading.curl}
      >
        {loading.curl ? "…" : "🔗"} cURL
      </button>

      <div className={styles.toolbarSeparator} />

      <button
        type="button"
        className={styles.toolbarButton}
        onClick={handleElementPick}
        disabled={!isConnected || loading.pick}
      >
        {loading.pick ? "Picking…" : "🎯 Pick Element"}
      </button>
      <button
        type="button"
        className={classNames(styles.toolbarButton, {
          [styles.toolbarButtonActive]:
            runtime?.attach_screenshot_on_send ?? false,
        })}
        onClick={handleToggleScreenshotOnSend}
        disabled={!isConnected}
      >
        📎 Auto-Screenshot
      </button>

      <div className={styles.toolbarSeparator} />

      <button
        type="button"
        className={styles.toolbarButton}
        onClick={handleRecordAnimation}
        disabled={!isConnected || loading.record}
      >
        {loading.record ? "Recording…" : "📽️ Record"}
      </button>
      <button
        type="button"
        className={styles.toolbarButton}
        onClick={handleSummarizePage}
        disabled={!isConnected || loading.summarize}
      >
        {loading.summarize ? "…" : "📝"} Summarize
      </button>
      <button
        type="button"
        className={styles.toolbarButton}
        onClick={handleExtractJson}
        disabled={!isConnected || loading.extract}
      >
        {loading.extract ? "…" : "📊"} Extract JSON
      </button>
    </div>
  );
};

import { useCallback } from "react";
import classNames from "classnames";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectBrowserRuntime,
  selectBrowserContextOversize,
  selectTimelineOpen,
  toggleTimelineOpen,
  setBrowserNotification,
  setBrowserRuntime,
  updateBrowserFrame,
  makeBrowserRuntime,
} from "./browserSlice";
import { browserApi } from "../../services/refact/browser";
import { BrowserToolbar } from "./BrowserToolbar";
import { BrowserContextGuard } from "./BrowserContextGuard";
import { ActionTimeline } from "./ActionTimeline";
import { useBrowserToolbarActions } from "./useBrowserToolbarActions";
import styles from "./Browser.module.css";

type BrowserPanelProps = {
  chatId: string;
};

export const BrowserPanel = ({ chatId }: BrowserPanelProps) => {
  const dispatch = useAppDispatch();
  const runtime = useAppSelector((state) =>
    selectBrowserRuntime(state, chatId),
  );
  const timelineOpen = useAppSelector((state) =>
    selectTimelineOpen(state, chatId),
  );
  const oversizeInfo = useAppSelector((state) =>
    selectBrowserContextOversize(state, chatId),
  );
  const [browserStart] = browserApi.useBrowserStartMutation();
  const [browserScreenshot] = browserApi.useBrowserScreenshotMutation();

  useBrowserToolbarActions(chatId);

  const isConnected = runtime?.connected ?? false;
  const url = runtime?.url ?? "";
  const frame = runtime?.latest_frame;
  const notification = runtime?.notification;

  const handleRestart = useCallback(() => {
    void (async () => {
      try {
        dispatch(setBrowserNotification({ chatId, notification: null }));
        const result = await browserStart({ chat_id: chatId }).unwrap();
        dispatch(setBrowserRuntime({ chatId, runtime: makeBrowserRuntime(result.runtime_id) }));
        const screenshotResult = await browserScreenshot({
          chat_id: chatId,
          full_page: false,
        }).unwrap();
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
        // Silently ignore restart failures
      }
    })();
  }, [browserStart, browserScreenshot, chatId, dispatch]);

  const handleDismissNotification = useCallback(() => {
    dispatch(setBrowserNotification({ chatId, notification: null }));
  }, [dispatch, chatId]);

  const handleToggleTimeline = useCallback(() => {
    dispatch(toggleTimelineOpen({ chatId }));
  }, [dispatch, chatId]);

  return (
    <div className={styles.browserPanel}>
      <BrowserToolbar chatId={chatId} />
      {notification && (
        <div
          className={classNames(styles.notification, {
            [styles.notificationDetached]: notification.type === "detached",
            [styles.notificationClosed]: notification.type === "closed",
            [styles.notificationTimeout]: notification.type === "timeout",
            [styles.notificationAttached]: notification.type === "attached",
          })}
        >
          <span>{notification.message}</span>
          {(notification.type === "closed" ||
            notification.type === "timeout") && (
            <button
              type="button"
              className={styles.restartButton}
              onClick={handleRestart}
            >
              Restart
            </button>
          )}
          <button
            type="button"
            className={styles.dismissButton}
            onClick={handleDismissNotification}
          >
            ✕
          </button>
        </div>
      )}
      <div className={styles.statusBar}>
        <span
          className={classNames(styles.statusDot, {
            [styles.statusDotConnected]: isConnected,
            [styles.statusDotDisconnected]: !isConnected,
          })}
        />
        <span className={styles.statusUrl}>
          {url || (isConnected ? "Connected" : "Not connected")}
        </span>
        <button
          type="button"
          className={classNames(styles.timelineToggle, {
            [styles.timelineToggleActive]: timelineOpen,
          })}
          onClick={handleToggleTimeline}
          data-testid="timeline-toggle"
        >
          Timeline
        </button>
      </div>
      {frame && (
        <div className={styles.frameContainer}>
          <img
            className={styles.frameImage}
            src={`data:${frame.mime};base64,${frame.data}`}
            alt="Browser frame"
          />
        </div>
      )}
      {!frame && isConnected && (
        <div className={styles.frameContainer}>
          <span className={styles.framePlaceholder}>
            Waiting for browser frame…
          </span>
        </div>
      )}
      {timelineOpen && <ActionTimeline chatId={chatId} />}
      {oversizeInfo && <BrowserContextGuard chatId={chatId} />}
    </div>
  );
};

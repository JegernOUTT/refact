import { useCallback } from "react";
import classNames from "classnames";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectBrowserRuntime,
  selectTimelineOpen,
  toggleTimelineOpen,
  setBrowserRuntime,
  setBrowserNotification,
} from "./browserSlice";
import { browserApi } from "../../services/refact/browser";
import { BrowserToolbar } from "./BrowserToolbar";
import { ActionTimeline } from "./ActionTimeline";
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
  const [browserStart] = browserApi.useBrowserStartMutation();

  const isConnected = runtime?.connected ?? false;
  const url = runtime?.url ?? "";
  const frame = runtime?.latest_frame;
  const notification = runtime?.notification;

  const handleRestart = useCallback(() => {
    void (async () => {
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
            notification: null,
          },
        }),
      );
    })();
  }, [browserStart, chatId, dispatch]);

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
    </div>
  );
};

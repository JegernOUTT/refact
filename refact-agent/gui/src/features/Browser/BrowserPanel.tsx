import { useCallback } from "react";
import classNames from "classnames";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectBrowserRuntime,
  selectTimelineOpen,
  toggleTimelineOpen,
} from "./browserSlice";
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

  const isConnected = runtime?.connected ?? false;
  const url = runtime?.url ?? "";
  const frame = runtime?.latest_frame;

  const handleToggleTimeline = useCallback(() => {
    dispatch(toggleTimelineOpen({ chatId }));
  }, [dispatch, chatId]);

  return (
    <div className={styles.browserPanel}>
      <BrowserToolbar chatId={chatId} />
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

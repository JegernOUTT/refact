import { Plus, X } from "lucide-react";
import { Badge, Button, IconButton, Tooltip } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import type { RootState } from "../../app/store";
import {
  selectBrowserViewOwners,
  selectFocusedWorkspaceChatId,
} from "../Workspace/workspaceSlice";
import {
  browserTabOwnerChatId,
  browserTabTitle,
  browserUrlLabel,
  nonBlank,
} from "./browserTabTitle";
import {
  openBrowserTabForChat,
  selectBrowserTabHintNames,
} from "./browserViews";
import { useProjectBrowserStatus } from "./useProjectBrowserStatus";
import { useProjectBrowserTabActions } from "./useProjectBrowserTabActions";
import styles from "./BrowserDockPanel.module.css";

const selectChatTitles = (state: RootState) => state.chat.threads;
const selectHistoryChats = (state: RootState) => state.history.chats;

export const BROWSER_DOCK_TASK_CHAT_NOTE =
  "Browser tabs can only be shown next to workspace chats. New tabs added here are still used by the focused chat.";

export function BrowserDockPanel() {
  const dispatch = useAppDispatch();
  const { status, error, loading, start, starting, startError } =
    useProjectBrowserStatus();
  const focusedChatId = useAppSelector(selectFocusedWorkspaceChatId);
  const viewChatId = useAppSelector((state) =>
    focusedChatId && state.chat.open_thread_ids.includes(focusedChatId)
      ? focusedChatId
      : null,
  );
  const owners = useAppSelector(selectBrowserViewOwners);
  const names = useAppSelector(selectBrowserTabHintNames);
  const threads = useAppSelector(selectChatTitles);
  const history = useAppSelector(selectHistoryChats);
  const actions = useProjectBrowserTabActions();
  const tabs = status?.tabs ?? [];

  const chatTitle = (chatId: string | null) => {
    if (!chatId) return "No chat";
    return (
      nonBlank(threads[chatId]?.thread.title) ??
      nonBlank(chatId in history ? history[chatId].title : undefined) ??
      "Untitled chat"
    );
  };

  return (
    <section
      id="workspace-browser-panel"
      aria-label="Browser"
      className={styles.panel}
    >
      <header className={styles.header}>
        <h2>Browser</h2>
        {status?.running && (
          <Badge
            size="xs"
            tone="muted"
            aria-label={`${tabs.length} browser tabs`}
          >
            {tabs.length}
          </Badge>
        )}
        <Tooltip
          content={
            focusedChatId
              ? "New tab next to the focused chat"
              : "Open a chat to add a browser tab"
          }
        >
          <IconButton
            className={styles.newTab}
            icon={Plus}
            size="sm"
            aria-label="New tab"
            disabled={!focusedChatId || actions.pending}
            onClick={() => {
              if (focusedChatId)
                void actions.createTabForChat(focusedChatId, status);
            }}
          />
        </Tooltip>
      </header>
      {actions.error && <p role="alert">{actions.error}</p>}
      {focusedChatId && !viewChatId && (
        <p className={styles.muted}>{BROWSER_DOCK_TASK_CHAT_NOTE}</p>
      )}
      {!status ? (
        error ? (
          <p role="alert">{error}</p>
        ) : loading ? (
          <p role="status">Checking project browser…</p>
        ) : null
      ) : !status.running ? (
        <div>
          {starting ? (
            <p role="status">Starting browser…</p>
          ) : (
            <>
              {startError && <p role="alert">{startError}</p>}
              <Button onClick={() => void start()}>Start browser</Button>
            </>
          )}
        </div>
      ) : tabs.length === 0 && status.busy ? (
        <p role="status">The browser is busy…</p>
      ) : tabs.length === 0 ? (
        <p className={styles.muted}>
          No browser tabs yet. Add one, or ask your assistant to browse.
        </p>
      ) : (
        <ul className={styles.list} aria-label="Browser tabs">
          {tabs.map((tab) => {
            const title = browserTabTitle({
              ...tab,
              name: tab.name ?? names[tab.tab_id],
            });
            const here = Boolean(
              viewChatId && owners[tab.tab_id] === viewChatId,
            );
            return (
              <li key={tab.tab_id} className={styles.item} data-here={here}>
                <button
                  type="button"
                  className={styles.open}
                  title={`Show ${title} next to the focused chat`}
                  disabled={!viewChatId}
                  onClick={() => {
                    if (viewChatId)
                      dispatch(
                        openBrowserTabForChat({
                          chatId: viewChatId,
                          tabId: tab.tab_id,
                        }),
                      );
                  }}
                >
                  <span className={styles.titleLine}>
                    <span className={styles.title} title={title}>
                      {title}
                    </span>
                    {here && (
                      <Badge size="xs" tone="accent">
                        Open here
                      </Badge>
                    )}
                  </span>
                  <span className={styles.detail} title={tab.url}>
                    {browserUrlLabel(tab.url) || tab.url}
                  </span>
                  <span className={styles.detail}>
                    {chatTitle(browserTabOwnerChatId(tab))}
                  </span>
                </button>
                <Tooltip content="Close the browser tab">
                  <IconButton
                    className={styles.close}
                    icon={X}
                    size="sm"
                    aria-label={`Close tab: ${title}`}
                    disabled={actions.pending}
                    onClick={() => void actions.closeBrowserTab(tab.tab_id)}
                  />
                </Tooltip>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

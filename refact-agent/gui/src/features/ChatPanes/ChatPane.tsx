import classNames from "classnames";
import { MessageSquare } from "lucide-react";
import { useCallback } from "react";

import { EmptyState } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { selectConfig } from "../Config/configSlice";
import { Chat } from "../Chat/Chat";
import { ChatThreadProvider } from "../Chat/Thread";
import { findLeaf } from "./panesTree";
import { focusPane, selectFocusedLeafId, selectPaneRoot } from "./panesSlice";
import { PaneTabStrip } from "./PaneTabStrip";
import styles from "./ChatPane.module.css";

export type ChatPaneProps = {
  leafId: string;
};

export function ChatPane({ leafId }: ChatPaneProps) {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const leaf = useAppSelector((state) =>
    findLeaf(selectPaneRoot(state), leafId),
  );
  const focusedLeafId = useAppSelector(selectFocusedLeafId);
  const activeTabId = leaf?.activeTabId ?? null;
  const focused = focusedLeafId === leafId;

  const handleFocusPane = useCallback(() => {
    dispatch(focusPane(leafId));
  }, [dispatch, leafId]);

  const handleBackFromChat = useCallback(() => undefined, []);

  if (!leaf) return null;

  return (
    <section
      className={classNames(styles.pane, focused && styles.focused)}
      aria-label={`Chat pane ${leafId}`}
      data-focused={focused || undefined}
      onClick={handleFocusPane}
      onFocusCapture={handleFocusPane}
    >
      <PaneTabStrip leafId={leafId} />
      <div className={styles.body}>
        {activeTabId ? (
          <ChatThreadProvider chatId={activeTabId}>
            <Chat
              host={config.host}
              tabbed={config.tabbed}
              backFromChat={handleBackFromChat}
              chatId={activeTabId}
            />
          </ChatThreadProvider>
        ) : (
          <div className={styles.emptyPane}>
            <EmptyState
              icon={MessageSquare}
              title="No chat selected"
              description="Open or drag a chat tab into this pane."
              variant="full"
            />
          </div>
        )}
      </div>
    </section>
  );
}

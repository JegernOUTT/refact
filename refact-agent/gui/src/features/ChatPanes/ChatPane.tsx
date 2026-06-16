import classNames from "classnames";
import { MessageSquare } from "lucide-react";
import { type DragEvent, useCallback, useState } from "react";

import { EmptyState } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { selectConfig } from "../Config/configSlice";
import { Chat } from "../Chat/Chat";
import { ChatThreadProvider } from "../Chat/Thread";
import { findLeaf } from "./panesTree";
import {
  focusPane,
  selectFocusedLeafId,
  selectPaneRoot,
  splitPane,
} from "./panesSlice";
import { PaneTabStrip } from "./PaneTabStrip";
import { hasTabDragType, readTabDragData } from "./tabDrag";
import styles from "./ChatPane.module.css";

export type ChatPaneProps = {
  leafId: string;
};

type PaneDropEdge = "left" | "right" | "top" | "bottom";

const paneDropEdges: PaneDropEdge[] = ["left", "right", "top", "bottom"];

const paneDropDirections: Record<PaneDropEdge, "row" | "col"> = {
  left: "row",
  right: "row",
  top: "col",
  bottom: "col",
};

const paneDropPlacements: Record<PaneDropEdge, "before" | "after"> = {
  left: "before",
  right: "after",
  top: "before",
  bottom: "after",
};

const paneDropEdgeClasses: Record<PaneDropEdge, string> = {
  left: styles.edgeDropLeft,
  right: styles.edgeDropRight,
  top: styles.edgeDropTop,
  bottom: styles.edgeDropBottom,
};

function readChatTabDrag(event: DragEvent): string | null {
  const dragged = readTabDragData(event.dataTransfer);
  return dragged?.type === "chat" ? dragged.id : null;
}

export function ChatPane({ leafId }: ChatPaneProps) {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const leaf = useAppSelector((state) =>
    findLeaf(selectPaneRoot(state), leafId),
  );
  const focusedLeafId = useAppSelector(selectFocusedLeafId);
  const activeTabId = leaf?.activeTabId ?? null;
  const focused = focusedLeafId === leafId;
  const [tabDragActive, setTabDragActive] = useState(false);

  const handleFocusPane = useCallback(() => {
    dispatch(focusPane(leafId));
  }, [dispatch, leafId]);

  const handleBackFromChat = useCallback(() => undefined, []);

  const handlePaneDragEnter = useCallback((event: DragEvent) => {
    if (!hasTabDragType(event.dataTransfer, "chat")) return;
    setTabDragActive(true);
  }, []);

  const handlePaneDragOver = useCallback((event: DragEvent) => {
    if (!hasTabDragType(event.dataTransfer, "chat")) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    setTabDragActive(true);
  }, []);

  const handlePaneDragLeave = useCallback((event: DragEvent<HTMLElement>) => {
    const nextTarget = event.relatedTarget;
    if (
      nextTarget instanceof Node &&
      event.currentTarget.contains(nextTarget)
    ) {
      return;
    }
    setTabDragActive(false);
  }, []);

  const handlePaneDrop = useCallback((event: DragEvent) => {
    if (hasTabDragType(event.dataTransfer, "chat")) {
      event.preventDefault();
    }
    setTabDragActive(false);
  }, []);

  const handleEdgeDragOver = useCallback((event: DragEvent) => {
    event.preventDefault();
    event.stopPropagation();
    event.dataTransfer.dropEffect = "move";
  }, []);

  const handleEdgeDrop = useCallback(
    (edge: PaneDropEdge, event: DragEvent) => {
      event.preventDefault();
      event.stopPropagation();
      setTabDragActive(false);
      const tabId = readChatTabDrag(event);
      if (!tabId) return;
      dispatch(
        splitPane({
          leafId,
          dir: paneDropDirections[edge],
          tabId,
          placement: paneDropPlacements[edge],
        }),
      );
    },
    [dispatch, leafId],
  );

  if (!leaf) return null;

  return (
    <section
      className={classNames(styles.pane, focused && styles.focused)}
      aria-label={`Chat pane ${leafId}`}
      data-focused={focused || undefined}
      onMouseDownCapture={handleFocusPane}
      onPointerDownCapture={handleFocusPane}
      onClick={handleFocusPane}
      onFocusCapture={handleFocusPane}
      onDragEnter={handlePaneDragEnter}
      onDragOver={handlePaneDragOver}
      onDragLeave={handlePaneDragLeave}
      onDragEnd={handlePaneDrop}
      onDrop={handlePaneDrop}
    >
      <PaneTabStrip leafId={leafId} />
      {tabDragActive ? (
        <div className={styles.edgeDropZones} aria-hidden="true">
          {paneDropEdges.map((edge) => (
            <div
              key={edge}
              className={classNames(
                styles.edgeDropZone,
                paneDropEdgeClasses[edge],
              )}
              data-testid={`pane-edge-drop-${leafId}-${edge}`}
              onDragOver={handleEdgeDragOver}
              onDrop={(event) => handleEdgeDrop(edge, event)}
            />
          ))}
        </div>
      ) : null}
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

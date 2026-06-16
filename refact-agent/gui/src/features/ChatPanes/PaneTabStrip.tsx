import classNames from "classnames";
import { X } from "lucide-react";
import {
  ComponentProps,
  DragEvent,
  KeyboardEvent,
  MouseEvent,
  PointerEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import {
  Badge,
  FieldText,
  Icon,
  StatusDot,
  Tabs as KitTabs,
} from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { updateChatTitleById } from "../History/historySlice";
import {
  closeThread,
  reorderOpenThreads,
  saveTitle,
  selectTabsDisplayData,
} from "../Chat/Thread";
import { getStatusFromSessionState } from "../../utils/sessionStatus";
import { useGetChatModesQuery } from "../../services/refact/chatModes";
import { findLeaf } from "./panesTree";
import {
  removeTabEverywhere,
  selectFocusedActiveTabId,
  selectFocusedLeafId,
  selectPaneRoot,
  setPaneActiveTab,
} from "./panesSlice";
import { parseTabDragData, tabDragData } from "./tabDrag";
import styles from "./PaneTabStrip.module.css";

export type PaneTabStripProps = {
  leafId: string;
};

function statusLabel(
  status: ComponentProps<typeof StatusDot>["status"],
): string {
  if (status === "in_progress" || status === "running") {
    return "In progress...";
  }

  if (status === "needs_attention" || status === "paused") {
    return "Needs your attention";
  }

  if (status === "error") {
    return "An error occurred";
  }

  if (status === "completed") {
    return "Completed";
  }

  return "Idle";
}

export function PaneTabStrip({ leafId }: PaneTabStripProps) {
  const dispatch = useAppDispatch();
  const activeTabRef = useRef<HTMLDivElement | null>(null);
  const leaf = useAppSelector((state) =>
    findLeaf(selectPaneRoot(state), leafId),
  );
  const focusedLeafId = useAppSelector(selectFocusedLeafId);
  const focusedActiveTabId = useAppSelector(selectFocusedActiveTabId);
  const tabs = useAppSelector(selectTabsDisplayData);
  const { data: modesData } = useGetChatModesQuery(undefined);

  const [draggingTabId, setDraggingTabId] = useState<string | null>(null);
  const [renameState, setRenameState] = useState<{
    id: string;
    value: string;
  } | null>(null);

  const activeTabId =
    focusedLeafId === leafId ? focusedActiveTabId : leaf?.activeTabId ?? null;

  const paneTabs = useMemo(() => {
    if (!leaf) return [];
    const leafTabIds = new Set(leaf.tabIds);
    return tabs.filter((tab) => leafTabIds.has(tab.id));
  }, [leaf, tabs]);

  useEffect(() => {
    if (activeTabRef.current?.scrollIntoView) {
      activeTabRef.current.scrollIntoView({
        behavior: "smooth",
        block: "nearest",
        inline: "nearest",
      });
    }
  }, [activeTabId]);

  const handleTabValueChange = useCallback(
    (tabId: string) => {
      dispatch(setPaneActiveTab({ leafId, tabId }));
    },
    [dispatch, leafId],
  );

  const handleCloseTab = useCallback(
    (event: MouseEvent, tabId: string) => {
      event.stopPropagation();
      event.preventDefault();
      dispatch(closeThread({ id: tabId }));
      dispatch(removeTabEverywhere(tabId));
    },
    [dispatch],
  );

  const handleMiddleClickClose = useCallback(
    (event: MouseEvent, tabId: string) => {
      if (event.button !== 1) return;
      handleCloseTab(event, tabId);
    },
    [handleCloseTab],
  );

  const handleChatThreadRenaming = useCallback(
    (tabId: string, currentTitle: string) => {
      setRenameState({ id: tabId, value: currentTitle });
    },
    [],
  );

  const handleRenameChange = useCallback((value: string) => {
    setRenameState((prev) => (prev ? { ...prev, value } : null));
  }, []);

  const handleKeyUpOnRename = useCallback(
    (event: KeyboardEvent<HTMLInputElement>, tabId: string) => {
      if (event.code === "Escape") {
        setRenameState(null);
      }
      if (event.code === "Enter") {
        const title = renameState?.value.trim();
        setRenameState(null);
        if (!title) return;
        dispatch(
          saveTitle({
            id: tabId,
            title,
            isTitleGenerated: true,
          }),
        );
        dispatch(updateChatTitleById({ chatId: tabId, newTitle: title }));
      }
    },
    [dispatch, renameState],
  );

  const stopClosePointerEvent = useCallback(
    (
      event: MouseEvent<HTMLButtonElement> | PointerEvent<HTMLButtonElement>,
    ) => {
      event.stopPropagation();
    },
    [],
  );

  const stopCloseDragEvent = useCallback(
    (event: DragEvent<HTMLButtonElement>) => {
      event.preventDefault();
      event.stopPropagation();
    },
    [],
  );

  const handleDragStart = useCallback((event: DragEvent, id: string) => {
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", tabDragData("chat", id));
    setDraggingTabId(id);
  }, []);

  const handleDragEnd = useCallback(() => {
    setDraggingTabId(null);
  }, []);

  const handleDragOver = useCallback((event: DragEvent) => {
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
  }, []);

  const handleDrop = useCallback(
    (event: DragEvent, id: string) => {
      event.preventDefault();
      const dragged = parseTabDragData(
        event.dataTransfer.getData("text/plain"),
      );
      if (!dragged || dragged.type !== "chat" || dragged.id === id) return;
      dispatch(reorderOpenThreads({ sourceId: dragged.id, targetId: id }));
    },
    [dispatch],
  );

  if (!leaf) {
    return null;
  }

  return (
    <div className={styles.paneTabStrip}>
      <KitTabs
        className={classNames(styles.tabsContainer, "scrollX")}
        value={activeTabId ?? undefined}
        onValueChange={handleTabValueChange}
        onWheel={(event) => {
          const container = event.currentTarget;
          if (container.scrollWidth <= container.clientWidth) return;
          event.preventDefault();
          container.scrollLeft += event.deltaY || event.deltaX;
        }}
      >
        <KitTabs.List className={styles.tabList} aria-label="Pane chat tabs">
          {paneTabs.map((tab) => {
            const isActive = activeTabId === tab.id;
            const isRenaming = renameState?.id === tab.id;

            if (isRenaming) {
              return (
                <div key={tab.id} className={styles.tabWrap}>
                  <FieldText
                    autoComplete="off"
                    onKeyUp={(event) => handleKeyUpOnRename(event, tab.id)}
                    onBlur={() => setRenameState(null)}
                    autoFocus
                    value={renameState.value}
                    onChange={handleRenameChange}
                    className={styles.renameInput}
                  />
                </div>
              );
            }

            const statusState = getStatusFromSessionState(tab.session_state);
            const modeInfo = modesData?.modes.find(
              (mode) => mode.id === tab.mode,
            );
            const modeLabel = modeInfo?.title ?? tab.mode;

            return (
              <div
                key={tab.id}
                className={classNames(
                  styles.tabWrap,
                  draggingTabId === tab.id && styles.tabWrapDragging,
                )}
                onDragOver={handleDragOver}
                onDrop={(event) => handleDrop(event, tab.id)}
                ref={isActive ? activeTabRef : undefined}
              >
                <KitTabs.Trigger value={tab.id} asChild>
                  <button
                    type="button"
                    aria-selected={isActive}
                    draggable
                    className={classNames(
                      styles.tabButton,
                      "rf-enter",
                      "rf-pressable",
                      isActive && styles.tabButtonActive,
                    )}
                    onAuxClick={(event) =>
                      handleMiddleClickClose(event, tab.id)
                    }
                    onDoubleClick={() =>
                      handleChatThreadRenaming(tab.id, tab.title)
                    }
                    onDragStart={(event) => handleDragStart(event, tab.id)}
                    onDragEnd={handleDragEnd}
                    title={tab.title}
                  >
                    <span className={styles.tabStatus}>
                      <StatusDot
                        aria-label={statusLabel(statusState)}
                        status={statusState}
                        size="small"
                      />
                    </span>
                    <span className={styles.tabTitle}>{tab.title}</span>
                    {tab.unreadNotificationCount > 0 && (
                      <span
                        className={styles.tabNotificationBadge}
                        aria-label={`${tab.unreadNotificationCount} unread process notifications`}
                      >
                        {tab.unreadNotificationCount > 9
                          ? "9+"
                          : tab.unreadNotificationCount}
                      </span>
                    )}
                    {!tab.is_buddy_chat && modeLabel && (
                      <Badge tone="muted" className={styles.tabModeBadge}>
                        {modeLabel}
                      </Badge>
                    )}
                  </button>
                </KitTabs.Trigger>
                <button
                  type="button"
                  className={styles.tabClose}
                  title="Close tab"
                  aria-label="Close tab"
                  draggable={false}
                  onMouseDown={stopClosePointerEvent}
                  onPointerDown={stopClosePointerEvent}
                  onDragStart={stopCloseDragEvent}
                  onClick={(event) => handleCloseTab(event, tab.id)}
                >
                  <Icon icon={X} size="sm" tone="muted" />
                </button>
              </div>
            );
          })}
        </KitTabs.List>
      </KitTabs>
    </div>
  );
}

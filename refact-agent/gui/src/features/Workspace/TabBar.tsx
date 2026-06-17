import classNames from "classnames";
import { Layers, X } from "lucide-react";
import {
  ComponentProps,
  DragEvent,
  MouseEvent,
  PointerEvent,
  WheelEvent,
  useCallback,
  useMemo,
  useState,
} from "react";

import { Badge, Icon, StatusDot } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectAllThreads,
  selectTabsDisplayData,
  type TabDisplayData,
} from "../Chat/Thread";
import {
  collectLeafIds,
  collectTabIds,
  findLeaf,
} from "../ChatPanes/panesTree";
import {
  readTabDragData,
  setTabDragData,
  type TabDragKind,
} from "../ChatPanes/tabDrag";
import {
  closeTab,
  reorderTabs,
  selectActiveTabId,
  selectTabs,
  selectWorkspaceGroups,
  setActiveTab,
  type PaneGroup,
} from "./workspaceSlice";
import {
  isChatSurface,
  makeSurfaceKey,
  parseSurfaceKey,
  type SurfaceKey,
} from "./surfaceKey";
import { getStatusFromSessionState } from "../../utils/sessionStatus";
import styles from "./TabBar.module.css";

type DisplayInfo = {
  title: string;
  session_state?: string;
  is_buddy_chat?: boolean;
  is_task_chat?: boolean;
  unreadNotificationCount: number;
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

function fallbackSurfaceTitle(surfaceKey: SurfaceKey): string {
  try {
    const parsed = parseSurfaceKey(surfaceKey);
    if (parsed.kind === "dashboard") return "Dashboard";
    const prefix = parsed.kind[0].toUpperCase();
    return `${prefix}${parsed.kind.slice(1)} ${parsed.id}`;
  } catch {
    return surfaceKey;
  }
}

function activeGroupSurfaceKey(group: PaneGroup): SurfaceKey | null {
  const focusedLeaf = findLeaf(group.root, group.focusedLeafId);
  if (focusedLeaf?.activeTabId) return focusedLeaf.activeTabId;
  if (focusedLeaf?.tabIds[0]) return focusedLeaf.tabIds[0];
  return collectTabIds(group.root)[0] ?? null;
}

function isSurfaceKind(type: TabDragKind): type is "chat" | "task" | "buddy" {
  return type === "chat" || type === "task" || type === "buddy";
}

function tabDragPayloadForSurface(surfaceKey: SurfaceKey): {
  type: TabDragKind;
  id: string;
} {
  try {
    const parsed = parseSurfaceKey(surfaceKey);
    if (parsed.kind === "dashboard") {
      return { type: "surface", id: surfaceKey };
    }
    return { type: parsed.kind, id: parsed.id };
  } catch {
    return { type: "surface", id: surfaceKey };
  }
}

function surfaceKeyFromDragData(
  payload: ReturnType<typeof readTabDragData>,
): SurfaceKey | null {
  if (!payload) return null;
  if (payload.surfaceKey) return payload.surfaceKey;
  if (payload.type === "surface") return payload.id;
  if (!isSurfaceKind(payload.type)) return null;
  return makeSurfaceKey(payload.type, payload.id);
}

function displayInfoForSurface(
  surfaceKey: SurfaceKey,
  tabsById: ReadonlyMap<string, TabDisplayData>,
  threads: ReturnType<typeof selectAllThreads>,
): DisplayInfo {
  if (!isChatSurface(surfaceKey)) {
    return {
      title: fallbackSurfaceTitle(surfaceKey),
      unreadNotificationCount: 0,
    };
  }

  const chatId = surfaceKey.slice("chat:".length);
  const tab = tabsById.get(chatId);
  if (tab) {
    return tab;
  }

  const runtime = threads[chatId];
  return {
    title: runtime?.thread.title ?? fallbackSurfaceTitle(surfaceKey),
    session_state: runtime?.session_state,
    is_buddy_chat: Boolean(runtime?.thread.buddy_meta?.is_buddy_chat),
    is_task_chat: Boolean(runtime?.thread.is_task_chat),
    unreadNotificationCount: 0,
  };
}

export function TabBar() {
  const dispatch = useAppDispatch();
  const tabs = useAppSelector(selectTabs);
  const activeTabId = useAppSelector(selectActiveTabId);
  const groups = useAppSelector(selectWorkspaceGroups);
  const tabDisplayData = useAppSelector(selectTabsDisplayData);
  const threads = useAppSelector(selectAllThreads);
  const [draggingTabId, setDraggingTabId] = useState<SurfaceKey | null>(null);
  const [dragTargetTabId, setDragTargetTabId] = useState<SurfaceKey | null>(
    null,
  );

  const tabsById = useMemo(
    () => new Map(tabDisplayData.map((tab) => [tab.id, tab])),
    [tabDisplayData],
  );

  const tabItems = useMemo(
    () =>
      tabs.map((tabId) => {
        const group = groups[tabId] ?? null;
        const isGroup = Boolean(group);
        const groupSurfaceKeys = group ? collectTabIds(group.root) : [tabId];
        const displaySurfaceKey = group
          ? activeGroupSurfaceKey(group) ?? tabId
          : tabId;
        const display = displayInfoForSurface(
          displaySurfaceKey,
          tabsById,
          threads,
        );
        const unreadNotificationCount = groupSurfaceKeys.reduce(
          (count, surfaceKey) =>
            count +
            displayInfoForSurface(surfaceKey, tabsById, threads)
              .unreadNotificationCount,
          0,
        );

        return {
          id: tabId,
          title: display.title,
          status: getStatusFromSessionState(display.session_state),
          unreadNotificationCount,
          is_buddy_chat: display.is_buddy_chat,
          is_task_chat: display.is_task_chat,
          isGroup,
          paneCount: group ? collectLeafIds(group.root).length : 1,
        };
      }),
    [groups, tabs, tabsById, threads],
  );

  const handleTabClick = useCallback(
    (tabId: SurfaceKey) => {
      dispatch(setActiveTab(tabId));
    },
    [dispatch],
  );

  const handleCloseTab = useCallback(
    (event: MouseEvent<HTMLButtonElement>, tabId: SurfaceKey) => {
      event.preventDefault();
      event.stopPropagation();
      dispatch(closeTab(tabId));
    },
    [dispatch],
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

  const handleDragStart = useCallback((event: DragEvent, tabId: SurfaceKey) => {
    event.dataTransfer.effectAllowed = "move";
    const payload = tabDragPayloadForSurface(tabId);
    setTabDragData(event.dataTransfer, payload.type, payload.id, tabId);
    setDraggingTabId(tabId);
  }, []);

  const handleDragEnd = useCallback(() => {
    setDraggingTabId(null);
    setDragTargetTabId(null);
  }, []);

  const handleTabDragOver = useCallback(
    (event: DragEvent, targetKey: SurfaceKey) => {
      const sourceKey = surfaceKeyFromDragData(
        readTabDragData(event.dataTransfer),
      );
      if (!sourceKey || sourceKey === targetKey || !tabs.includes(sourceKey)) {
        return;
      }
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      setDragTargetTabId(targetKey);
    },
    [tabs],
  );

  const handleTabDragLeave = useCallback(
    (event: DragEvent<HTMLElement>, targetKey: SurfaceKey) => {
      const related = event.relatedTarget;
      if (related instanceof Node && event.currentTarget.contains(related)) {
        return;
      }
      setDragTargetTabId((current) => (current === targetKey ? null : current));
    },
    [],
  );

  const handleTabDrop = useCallback(
    (event: DragEvent, targetKey: SurfaceKey) => {
      event.preventDefault();
      event.stopPropagation();
      const sourceKey = surfaceKeyFromDragData(
        readTabDragData(event.dataTransfer),
      );
      setDragTargetTabId(null);
      if (!sourceKey || sourceKey === targetKey) return;
      dispatch(reorderTabs({ sourceKey, targetKey }));
    },
    [dispatch],
  );

  const handleBarDragOver = useCallback((event: DragEvent) => {
    if (!readTabDragData(event.dataTransfer)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
  }, []);

  const handleBarDrop = useCallback((event: DragEvent) => {
    if (!readTabDragData(event.dataTransfer)) return;
    event.preventDefault();
    setDragTargetTabId(null);
  }, []);

  const handleWheel = useCallback((event: WheelEvent<HTMLElement>) => {
    const container = event.currentTarget;
    if (container.scrollWidth <= container.clientWidth) return;
    event.preventDefault();
    container.scrollLeft += event.deltaY || event.deltaX;
  }, []);

  return (
    <nav className={styles.tabBar} aria-label="Workspace tabs">
      <div
        className={classNames(styles.scrollArea, "scrollX")}
        onWheel={handleWheel}
        onDragOver={handleBarDragOver}
        onDrop={handleBarDrop}
      >
        <div
          className={classNames(styles.tabList, "rf-stagger")}
          role="tablist"
          aria-label="Open workspace tabs"
        >
          {tabItems.map((tab) => {
            const isActive = activeTabId === tab.id;
            const unreadText =
              tab.unreadNotificationCount > 9
                ? "9+"
                : tab.unreadNotificationCount;
            const tabTitle = tab.isGroup
              ? `${tab.title} · ${tab.paneCount} panes`
              : tab.title;

            return (
              <div
                key={tab.id}
                className={classNames(
                  styles.tabWrap,
                  "rf-enter-scale",
                  isActive && styles.tabWrapActive,
                  tab.isGroup && styles.tabWrapGroup,
                  draggingTabId === tab.id && styles.tabWrapDragging,
                  dragTargetTabId === tab.id && styles.tabWrapDropTarget,
                )}
                onDragOver={(event) => handleTabDragOver(event, tab.id)}
                onDragLeave={(event) => handleTabDragLeave(event, tab.id)}
                onDrop={(event) => handleTabDrop(event, tab.id)}
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={isActive}
                  className={styles.tabButton}
                  draggable
                  onClick={() => handleTabClick(tab.id)}
                  onDragStart={(event) => handleDragStart(event, tab.id)}
                  onDragEnd={handleDragEnd}
                  title={tabTitle}
                >
                  <span className={styles.tabStatus}>
                    <StatusDot
                      aria-label={statusLabel(tab.status)}
                      status={tab.status}
                      size="small"
                    />
                  </span>
                  {tab.isGroup && (
                    <Badge
                      tone="accent"
                      size="xs"
                      variant="outline"
                      className={styles.groupBadge}
                      aria-label={`${tab.paneCount} panes`}
                    >
                      <Icon icon={Layers} size="sm" />
                      {tab.paneCount}
                    </Badge>
                  )}
                  {tab.is_buddy_chat && (
                    <Badge
                      tone="accent"
                      size="xs"
                      variant="outline"
                      className={styles.kindBadge}
                    >
                      Buddy
                    </Badge>
                  )}
                  {tab.is_task_chat && (
                    <Badge
                      tone="muted"
                      size="xs"
                      variant="outline"
                      className={styles.kindBadge}
                    >
                      Task
                    </Badge>
                  )}
                  <span className={styles.tabTitle}>{tab.title}</span>
                  {tab.unreadNotificationCount > 0 && (
                    <Badge
                      tone="warning"
                      size="xs"
                      className={styles.notificationBadge}
                      aria-label={`${tab.unreadNotificationCount} unread process notifications`}
                    >
                      {unreadText}
                    </Badge>
                  )}
                </button>
                <button
                  type="button"
                  className={styles.tabClose}
                  title="Close tab"
                  aria-label={`Close ${tab.title}`}
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
        </div>
      </div>
    </nav>
  );
}

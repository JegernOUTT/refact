import { displayInfoForSurface } from "./tabDisplay";
import classNames from "classnames";
import {
  AppWindow,
  FileDiff,
  GitBranch,
  FileText,
  Layers,
  Menu as MenuIcon,
  Palette,
  X,
} from "lucide-react";
import {
  DragEvent,
  MouseEvent,
  PointerEvent,
  TransitionEvent as ReactTransitionEvent,
  WheelEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { Badge, Icon, IconButton } from "../../components/ui";
import { useAppDispatch, useAppSelector, useReducedMotion } from "../../hooks";
import { selectCapabilities, selectHost } from "../Config/configSlice";
import {
  popBackTo,
  push,
  selectCurrentPage,
  selectPages,
} from "../Pages/pagesSlice";
import { selectAllThreads, selectTabsDisplayData } from "../Chat/Thread";
import {
  collectLeafIds,
  collectTabIds,
  findLeaf,
} from "../ChatPanes/panesTree";
import {
  hasTabDragType,
  readTabDragData,
  setTabDragData,
  surfaceKeyFromTabDragPayload,
  type TabDragPayload,
  type TabDragKind,
} from "../ChatPanes/tabDrag";
import {
  beginPointerDragGesture,
  pointerDragController,
  rectFromElement,
} from "../ChatPanes/pointerDrag";
import { usePointerDragHost } from "../ChatPanes/usePointerDrag";
import {
  closeTask,
  reorderOpenTasks,
  selectOpenTasksFromRoot,
} from "../Tasks/tasksSlice";
import {
  closeTab,
  reorderTabs,
  selectActiveTabId,
  selectPanelsForced,
  selectTabs,
  selectWorkspaceGroups,
  setActiveTab,
  selectWorkspaceDock,
  toggleDock,
  type PaneGroup,
} from "./workspaceSlice";
import {
  isMainSurfaceKind,
  makeSurfaceKey,
  parseSurfaceKey,
  type SurfaceKey,
} from "./surfaceKey";
import { resolveWorkspaceDockAvailability } from "./workspaceAvailability";
import { activityFromSessionState } from "../../utils/chatActivity";
import { ChatActivityStatus } from "../../components/ChatActivity/ChatActivityStatus";
import styles from "./TabBar.module.css";

const BUDDY_SURFACE_KEY = makeSurfaceKey("buddy", "home");
const TAB_CLOSE_FALLBACK_MS = 250;

function activeGroupSurfaceKey(group: PaneGroup): SurfaceKey | null {
  const focusedLeaf = findLeaf(group.root, group.focusedLeafId);
  if (focusedLeaf?.activeTabId) return focusedLeaf.activeTabId;
  if (focusedLeaf?.tabIds[0]) return focusedLeaf.tabIds[0];
  return collectTabIds(group.root)[0] ?? null;
}

function uniqueSurfaceKeys(keys: SurfaceKey[]): SurfaceKey[] {
  const seen = new Set<SurfaceKey>();
  return keys.filter((key) => {
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function tabDragPayloadForSurface(surfaceKey: SurfaceKey): {
  type: TabDragKind;
  id: string;
} {
  try {
    const parsed = parseSurfaceKey(surfaceKey);
    if (
      parsed.kind === "dashboard" ||
      parsed.kind === "app" ||
      parsed.kind === "design" ||
      parsed.kind === "gitdiff" ||
      parsed.kind === "worktreediff" ||
      isMainSurfaceKind(parsed.kind)
    ) {
      return { type: "surface", id: surfaceKey };
    }
    return { type: parsed.kind, id: parsed.id };
  } catch {
    return { type: "surface", id: surfaceKey };
  }
}

function sameKindLabel(title: string, kind: string): boolean {
  return title.trim().toLowerCase() === kind.toLowerCase();
}

export type TabBarProps = {
  placement?: "workspace" | "toolbar";
};

export function TabBar({ placement = "workspace" }: TabBarProps) {
  const dispatch = useAppDispatch();
  const tabs = useAppSelector(selectTabs);
  const activeTabId = useAppSelector(selectActiveTabId);
  const groups = useAppSelector(selectWorkspaceGroups);
  const tabDisplayData = useAppSelector(selectTabsDisplayData);
  const threads = useAppSelector(selectAllThreads);
  const openTasks = useAppSelector(selectOpenTasksFromRoot);
  const currentPage = useAppSelector(selectCurrentPage);
  const pages = useAppSelector(selectPages);
  const capabilities = useAppSelector(selectCapabilities);
  const host = useAppSelector(selectHost);
  const panelsForced = useAppSelector(selectPanelsForced);
  const dock = useAppSelector(selectWorkspaceDock);
  const reducedMotion = useReducedMotion();
  const [draggingTabId, setDraggingTabId] = useState<SurfaceKey | null>(null);
  const [dragTargetTabId, setDragTargetTabId] = useState<SurfaceKey | null>(
    null,
  );
  const toolbarPlacement = placement === "toolbar";
  const pointerDragEnabled = usePointerDragHost();
  const [pointerDropTargetId, setPointerDropTargetId] =
    useState<SurfaceKey | null>(null);
  const [closingTabIds, setClosingTabIds] = useState<Set<SurfaceKey>>(
    () => new Set(),
  );
  const closingTabIdsRef = useRef(new Set<SurfaceKey>());
  const closeTimersRef = useRef(new Map<SurfaceKey, number>());
  const tabWrapEls = useRef(new Map<SurfaceKey, HTMLElement>());
  const tabListEl = useRef<HTMLDivElement | null>(null);
  const gestureCleanupRef = useRef<(() => void) | null>(null);

  const tabsById = useMemo(
    () => new Map(tabDisplayData.map((tab) => [tab.id, tab])),
    [tabDisplayData],
  );

  const tasksById = useMemo(
    () => new Map(openTasks.map((task) => [task.id, task])),
    [openTasks],
  );

  const taskSurfaceKeys = useMemo(
    () => openTasks.map((task) => makeSurfaceKey("task", task.id)),
    [openTasks],
  );

  const currentTaskSurfaceKey =
    currentPage?.name === "task workspace"
      ? makeSurfaceKey("task", currentPage.taskId)
      : null;
  const buddySurfaceOpen = pages.some((page) => page.name === "buddy");
  const { dock: dockAvailable } = resolveWorkspaceDockAvailability(
    host,
    capabilities,
    panelsForced,
  );
  const showDockToggle = !toolbarPlacement && host === "web" && dockAvailable;

  const visibleTabKeys = useMemo(
    () =>
      uniqueSurfaceKeys([
        ...tabs,
        ...taskSurfaceKeys,
        ...(currentTaskSurfaceKey ? [currentTaskSurfaceKey] : []),
        ...(buddySurfaceOpen ? [BUDDY_SURFACE_KEY] : []),
      ]),
    [buddySurfaceOpen, currentTaskSurfaceKey, tabs, taskSurfaceKeys],
  );

  const activeSurfaceKey = useMemo(() => {
    if (currentPage?.name === "task workspace") {
      return makeSurfaceKey("task", currentPage.taskId);
    }
    if (currentPage?.name === "buddy") return BUDDY_SURFACE_KEY;
    return activeTabId;
  }, [activeTabId, currentPage]);

  const tabItems = useMemo(
    () =>
      visibleTabKeys.map((tabId) => {
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
          tasksById,
        );
        const unreadNotificationCount = groupSurfaceKeys.reduce(
          (count, surfaceKey) =>
            count +
            displayInfoForSurface(surfaceKey, tabsById, threads, tasksById)
              .unreadNotificationCount,
          0,
        );

        return {
          id: tabId,
          title: display.title,
          tooltip: display.tooltip,
          kind: display.kind,
          activity:
            display.activity ?? activityFromSessionState(display.session_state),
          unreadNotificationCount,
          is_buddy_chat: display.is_buddy_chat,
          is_task_chat: display.is_task_chat,
          isGroup,
          paneCount: group ? collectLeafIds(group.root).length : 1,
          draggable: !isGroup,
          closable: display.kind !== "buddy" || currentPage?.name === "buddy",
        };
      }),
    [currentPage?.name, groups, tabsById, tasksById, threads, visibleTabKeys],
  );

  const handleTabClick = useCallback(
    (tabId: SurfaceKey) => {
      const parsed = parseSurfaceKey(tabId);
      if (parsed.kind === "chat") {
        dispatch(setActiveTab(tabId));
        if (currentPage?.name !== "chat") {
          dispatch(push({ name: "chat" }));
        }
        return;
      }
      if (parsed.kind === "task") {
        if (
          currentPage?.name !== "task workspace" ||
          currentPage.taskId !== parsed.id
        ) {
          dispatch(push({ name: "task workspace", taskId: parsed.id }));
        }
        return;
      }
      if (parsed.kind === "buddy") {
        if (currentPage?.name !== "buddy") {
          dispatch(push({ name: "buddy" }));
        }
        return;
      }
      if (
        parsed.kind === "app" ||
        parsed.kind === "design" ||
        parsed.kind === "file" ||
        parsed.kind === "gitdiff" ||
        parsed.kind === "worktreediff" ||
        isMainSurfaceKind(parsed.kind)
      ) {
        dispatch(setActiveTab(tabId));
        if (currentPage?.name !== "chat") {
          dispatch(push({ name: "chat" }));
        }
      }
    },
    [currentPage, dispatch],
  );

  const closeTabById = useCallback(
    (tabId: SurfaceKey) => {
      const parsed = parseSurfaceKey(tabId);
      if (parsed.kind === "task") {
        dispatch(closeTask(parsed.id));
        if (
          currentPage?.name === "task workspace" &&
          currentPage.taskId === parsed.id
        ) {
          dispatch(popBackTo({ name: "history" }));
        }
        return;
      }
      if (parsed.kind === "buddy") {
        if (currentPage?.name === "buddy") {
          dispatch(popBackTo({ name: "history" }));
        }
        return;
      }
      const wasLastWorkspaceTab = tabs.includes(tabId) && tabs.length <= 1;
      dispatch(closeTab(tabId));
      if (wasLastWorkspaceTab && currentPage?.name === "chat") {
        dispatch(popBackTo({ name: "history" }));
      }
    },
    [currentPage, dispatch, tabs],
  );

  const closeTabByIdRef = useRef(closeTabById);
  closeTabByIdRef.current = closeTabById;

  const clearCloseTimer = useCallback((tabId: SurfaceKey) => {
    const timer = closeTimersRef.current.get(tabId);
    if (timer === undefined) return;
    window.clearTimeout(timer);
    closeTimersRef.current.delete(tabId);
  }, []);

  const finalizeTabClose = useCallback(
    (tabId: SurfaceKey) => {
      if (!closingTabIdsRef.current.delete(tabId)) return;
      clearCloseTimer(tabId);
      setClosingTabIds((current) => {
        if (!current.has(tabId)) return current;
        const next = new Set(current);
        next.delete(tabId);
        return next;
      });
      closeTabByIdRef.current(tabId);
    },
    [clearCloseTimer],
  );

  const handleCloseTab = useCallback(
    (event: MouseEvent<HTMLButtonElement>, tabId: SurfaceKey) => {
      event.preventDefault();
      event.stopPropagation();
      if (reducedMotion) {
        closeTabById(tabId);
        return;
      }
      if (closingTabIdsRef.current.has(tabId)) return;
      closingTabIdsRef.current.add(tabId);
      setClosingTabIds((current) => {
        const next = new Set(current);
        next.add(tabId);
        return next;
      });
      const timer = window.setTimeout(
        () => finalizeTabClose(tabId),
        TAB_CLOSE_FALLBACK_MS,
      );
      closeTimersRef.current.set(tabId, timer);
    },
    [closeTabById, finalizeTabClose, reducedMotion],
  );

  const handleTabCloseTransitionEnd = useCallback(
    (event: ReactTransitionEvent<HTMLDivElement>, tabId: SurfaceKey) => {
      if (event.target !== event.currentTarget) return;
      finalizeTabClose(tabId);
    },
    [finalizeTabClose],
  );

  useEffect(() => {
    if (!reducedMotion) return;
    for (const tabId of Array.from(closingTabIdsRef.current)) {
      finalizeTabClose(tabId);
    }
  }, [finalizeTabClose, reducedMotion]);

  useEffect(() => {
    const tabList = tabListEl.current;
    if (!tabList) return;
    const handleTransitionCancel = (event: globalThis.TransitionEvent) => {
      const target = event.target;
      if (!(target instanceof HTMLElement)) return;
      const tabId = target.dataset.tabSurfaceKey;
      if (!tabId) return;
      finalizeTabClose(tabId);
    };
    tabList.addEventListener("transitioncancel", handleTransitionCancel);
    return () => {
      tabList.removeEventListener("transitioncancel", handleTransitionCancel);
    };
  }, [finalizeTabClose]);

  useEffect(() => {
    const visibleKeys = new Set(visibleTabKeys);
    const removedIds = Array.from(closingTabIdsRef.current).filter(
      (tabId) => !visibleKeys.has(tabId),
    );
    if (removedIds.length === 0) return;
    for (const tabId of removedIds) {
      closingTabIdsRef.current.delete(tabId);
      clearCloseTimer(tabId);
    }
    setClosingTabIds((current) => {
      const next = new Set(current);
      for (const tabId of removedIds) next.delete(tabId);
      return next;
    });
  }, [clearCloseTimer, visibleTabKeys]);

  useEffect(
    () => () => {
      for (const timer of closeTimersRef.current.values()) {
        window.clearTimeout(timer);
      }
      closeTimersRef.current.clear();
      closingTabIdsRef.current.clear();
    },
    [],
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

  const handleDragStart = useCallback(
    (event: DragEvent, tabId: SurfaceKey) => {
      if (groups[tabId]) {
        event.preventDefault();
        return;
      }

      event.dataTransfer.effectAllowed = "move";
      const payload = tabDragPayloadForSurface(tabId);
      setTabDragData(event.dataTransfer, payload.type, payload.id, tabId);
      setDraggingTabId(tabId);
    },
    [groups],
  );

  const handleDragEnd = useCallback(() => {
    setDraggingTabId(null);
    setDragTargetTabId(null);
  }, []);

  const handleTabDragOver = useCallback(
    (event: DragEvent, targetKey: SurfaceKey) => {
      const target = parseSurfaceKey(targetKey);
      const sourceKey = surfaceKeyFromTabDragPayload(
        readTabDragData(event.dataTransfer),
      );
      const workspaceReorder =
        tabs.includes(targetKey) &&
        sourceKey !== null &&
        tabs.includes(sourceKey);
      const taskReorder =
        target.kind === "task" &&
        taskSurfaceKeys.includes(targetKey) &&
        hasTabDragType(event.dataTransfer, "task");
      if (!workspaceReorder && !taskReorder) {
        return;
      }
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      setDragTargetTabId(targetKey);
    },
    [tabs, taskSurfaceKeys],
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

  const reorderFromKeys = useCallback(
    (sourceKey: SurfaceKey | null, targetKey: SurfaceKey) => {
      if (!sourceKey || sourceKey === targetKey) return;
      if (tabs.includes(sourceKey) && tabs.includes(targetKey)) {
        dispatch(reorderTabs({ sourceKey, targetKey }));
        return;
      }
      if (
        taskSurfaceKeys.includes(sourceKey) &&
        taskSurfaceKeys.includes(targetKey)
      ) {
        const source = parseSurfaceKey(sourceKey);
        const target = parseSurfaceKey(targetKey);
        if (source.kind === "task" && target.kind === "task") {
          dispatch(
            reorderOpenTasks({ sourceId: source.id, targetId: target.id }),
          );
        }
      }
    },
    [dispatch, tabs, taskSurfaceKeys],
  );

  const handleTabDrop = useCallback(
    (event: DragEvent, targetKey: SurfaceKey) => {
      event.preventDefault();
      event.stopPropagation();
      const sourceKey = surfaceKeyFromTabDragPayload(
        readTabDragData(event.dataTransfer),
      );
      setDragTargetTabId(null);
      reorderFromKeys(sourceKey, targetKey);
    },
    [reorderFromKeys],
  );

  const acceptsReorder = useCallback(
    (payload: TabDragPayload, targetKey: SurfaceKey) => {
      const sourceKey = surfaceKeyFromTabDragPayload(payload);
      if (!sourceKey || sourceKey === targetKey) return false;
      if (tabs.includes(sourceKey) && tabs.includes(targetKey)) {
        return tabs.includes(sourceKey) && tabs.includes(targetKey);
      }
      if (payload.type === "task") {
        return (
          taskSurfaceKeys.includes(sourceKey) &&
          taskSurfaceKeys.includes(targetKey)
        );
      }
      return false;
    },
    [tabs, taskSurfaceKeys],
  );

  const handleTabPointerDown = useCallback(
    (
      event: PointerEvent<HTMLButtonElement>,
      tab: { id: SurfaceKey; title: string; draggable: boolean },
    ) => {
      if (!pointerDragEnabled || !tab.draggable) return;
      gestureCleanupRef.current?.();
      const payload = tabDragPayloadForSurface(tab.id);
      gestureCleanupRef.current = beginPointerDragGesture(
        {
          button: event.button,
          clientX: event.clientX,
          clientY: event.clientY,
          pointerId: event.pointerId,
        },
        () => ({
          payload: { type: payload.type, id: payload.id, surfaceKey: tab.id },
          label: tab.title,
        }),
      );
    },
    [pointerDragEnabled],
  );

  useEffect(() => () => gestureCleanupRef.current?.(), []);

  useEffect(() => {
    if (!pointerDragEnabled) {
      setPointerDropTargetId(null);
      return;
    }

    const reorderableKeys = Array.from(
      new Set<SurfaceKey>([...tabs, ...taskSurfaceKeys]),
    );
    const unregisters = reorderableKeys.map((targetKey) =>
      pointerDragController.registerZone({
        id: `tabbar:${targetKey}`,
        getRect: () =>
          rectFromElement(tabWrapEls.current.get(targetKey) ?? null),
        accepts: (payload) => acceptsReorder(payload, targetKey),
        onDrop: (payload) =>
          reorderFromKeys(surfaceKeyFromTabDragPayload(payload), targetKey),
        setHover: (over) =>
          setPointerDropTargetId((current) =>
            over ? targetKey : current === targetKey ? null : current,
          ),
      }),
    );

    return () => {
      for (const unregister of unregisters) unregister();
    };
  }, [
    pointerDragEnabled,
    tabs,
    taskSurfaceKeys,
    acceptsReorder,
    reorderFromKeys,
  ]);

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
    <nav
      className={classNames(
        styles.tabBar,
        toolbarPlacement && styles.toolbarTabBar,
      )}
      aria-label="Workspace tabs"
    >
      {showDockToggle ? (
        <IconButton
          aria-label="Toggle workspace dock"
          aria-pressed={dock.open}
          className={styles.dockToggle}
          icon={MenuIcon}
          onClick={() => dispatch(toggleDock())}
          size="sm"
          variant="plain"
        />
      ) : null}
      <div
        className={classNames(styles.scrollArea, "scrollX")}
        onWheel={handleWheel}
        onDragOver={handleBarDragOver}
        onDrop={handleBarDrop}
      >
        <div
          ref={tabListEl}
          className={classNames(styles.tabList, "rf-stagger")}
          role="tablist"
          aria-label="Open workspace tabs"
        >
          {tabItems.map((tab) => {
            const isActive = activeSurfaceKey === tab.id;
            const isClosing = closingTabIds.has(tab.id);
            const unreadText =
              tab.unreadNotificationCount > 9
                ? "9+"
                : tab.unreadNotificationCount;
            const tabTitle = tab.isGroup
              ? `${tab.title} · ${tab.paneCount} panes`
              : tab.tooltip ?? tab.title;

            return (
              <div
                key={tab.id}
                data-tab-surface-key={tab.id}
                ref={(node) => {
                  if (node) tabWrapEls.current.set(tab.id, node);
                  else tabWrapEls.current.delete(tab.id);
                }}
                className={classNames(
                  styles.tabWrap,
                  !isClosing && "rf-enter-scale",
                  isActive && styles.tabWrapActive,
                  tab.isGroup && styles.tabWrapGroup,
                  isClosing && styles.tabWrapClosing,
                  draggingTabId === tab.id && styles.tabWrapDragging,
                  (dragTargetTabId === tab.id ||
                    pointerDropTargetId === tab.id) &&
                    styles.tabWrapDropTarget,
                )}
                onDragOver={(event) => handleTabDragOver(event, tab.id)}
                onDragLeave={(event) => handleTabDragLeave(event, tab.id)}
                onDrop={(event) => handleTabDrop(event, tab.id)}
                onTransitionEnd={(event) =>
                  handleTabCloseTransitionEnd(event, tab.id)
                }
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={isActive}
                  className={styles.tabButton}
                  draggable={tab.draggable && !pointerDragEnabled}
                  onClick={() => handleTabClick(tab.id)}
                  onPointerDown={(event) => handleTabPointerDown(event, tab)}
                  onDragStart={(event) => handleDragStart(event, tab.id)}
                  onDragEnd={handleDragEnd}
                  title={tabTitle}
                >
                  <span className={styles.tabStatus}>
                    {tab.kind === "file" ? (
                      <Icon icon={FileText} size="sm" tone="muted" />
                    ) : tab.kind === "gitdiff" ||
                      tab.kind === "worktreediff" ? (
                      <Icon icon={FileDiff} size="sm" tone="muted" />
                    ) : tab.kind === "git" ? (
                      <Icon icon={GitBranch} size="sm" tone="muted" />
                    ) : tab.kind === "app" ? (
                      <Icon icon={AppWindow} size="sm" tone="muted" />
                    ) : tab.kind === "design" ? (
                      <Icon icon={Palette} size="sm" tone="muted" />
                    ) : (
                      <ChatActivityStatus activity={tab.activity} compact />
                    )}
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
                  {(tab.kind === "buddy" || tab.is_buddy_chat) &&
                    !sameKindLabel(tab.title, "Buddy") && (
                      <Badge
                        tone="accent"
                        size="xs"
                        variant="outline"
                        className={styles.kindBadge}
                      >
                        Buddy
                      </Badge>
                    )}
                  {(tab.kind === "task" || tab.is_task_chat) &&
                    !sameKindLabel(tab.title, "Task") && (
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
                {tab.closable ? (
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
                ) : null}
              </div>
            );
          })}
        </div>
      </div>
    </nav>
  );
}

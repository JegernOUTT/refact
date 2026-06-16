import { Dropdown, DropdownNavigationOptions } from "./Dropdown";
import { CheckSquare, Moon, Plus, Sun, X, Home } from "lucide-react";
import classNames from "classnames";
import { newChatAction } from "../../events";
import { getTaskStatusDotState } from "../../utils/sessionStatus";
import { popBackTo, push } from "../../features/Pages/pagesSlice";
import {
  useCreateTaskMutation,
  useUpdateTaskMetaMutation,
  useListTasksQuery,
} from "../../services/refact/tasks";
import {
  selectOpenTasksFromRoot,
  openTask,
  closeTask,
  reorderOpenTasks,
} from "../../features/Tasks";
import {
  ComponentProps,
  KeyboardEvent,
  MouseEvent,
  PointerEvent,
  DragEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  selectAllThreads,
  closeThread,
  switchToThread,
  selectChatId,
  clearThreadPauseReasons,
  setThreadConfirmationStatus,
} from "../../features/Chat/Thread";
import {
  FieldText,
  Icon,
  IconButton,
  StatusDot,
  Tabs as KitTabs,
  Tooltip,
} from "../ui";
import {
  useAppDispatch,
  useAppSelector,
  useAppearance,
  useConfig,
  useEventsBusForIDE,
  useOpenUrl,
} from "../../hooks";
import {
  resolveEngineBaseUrl,
  type EngineApiConfig,
} from "../../services/refact/apiUrl";
import {
  parseTabDragData,
  tabDragData,
} from "../../features/ChatPanes/tabDrag";

import styles from "./Toolbar.module.css";
import { ConnectionStatusIndicator } from "../ConnectionStatus";

export type DashboardTab = {
  type: "dashboard";
};

export type ChatTab = {
  type: "chat";
  id: string;
};

function isChatTab(tab: Tab): tab is ChatTab {
  return tab.type === "chat";
}

export type TaskTab = {
  type: "task";
  taskId: string;
  taskName: string;
};

function isTaskTab(tab: Tab): tab is TaskTab {
  return tab.type === "task";
}

export type Tab = DashboardTab | ChatTab | TaskTab;

export type ToolbarProps = {
  activeTab: Tab;
};

type ToolbarIconButtonProps = {
  label: string;
  onClick: () => void;
  icon: ComponentProps<typeof IconButton>["icon"];
  className?: string;
  disabled?: boolean;
};

const ToolbarIconButton = ({
  label,
  onClick,
  icon,
  className,
  disabled,
}: ToolbarIconButtonProps) => (
  <Tooltip>
    <Tooltip.Trigger asChild>
      <IconButton
        aria-label={label}
        className={classNames(styles.iconButton, "rf-pressable", className)}
        disabled={disabled}
        icon={icon}
        onClick={onClick}
        size="sm"
        variant="plain"
      />
    </Tooltip.Trigger>
    <Tooltip.Content side="bottom">{label}</Tooltip.Content>
  </Tooltip>
);

function taskStatusLabel(
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

function isUsableHttpUrl(value: string | undefined): value is string {
  if (!value) return false;
  try {
    const url = new URL(value);
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

function normalizeDisplayUrl(value: string): string {
  return value.replace(/\/+$/, "");
}

function isLocalhostUrl(value: string): boolean {
  try {
    const { hostname } = new URL(value);
    return (
      hostname === "localhost" || hostname === "127.0.0.1" || hostname === "::1"
    );
  } catch {
    return false;
  }
}

function resolveCommonBrowserUrl(config: EngineApiConfig): string | null {
  if (isUsableHttpUrl(config.browserUrl)) {
    return normalizeDisplayUrl(config.browserUrl);
  }

  const candidates = window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ ?? [];
  const mdnsCandidate = candidates.find((candidate) => {
    if (!isUsableHttpUrl(candidate)) return false;
    return new URL(candidate).hostname.endsWith(".local");
  });
  if (mdnsCandidate) return normalizeDisplayUrl(mdnsCandidate);

  const lanCandidate = candidates.find((candidate) => {
    if (!isUsableHttpUrl(candidate)) return false;
    return !isLocalhostUrl(candidate);
  });
  if (lanCandidate) return normalizeDisplayUrl(lanCandidate);

  return null;
}

function resolveBrowserEngineUrl(config: EngineApiConfig): string {
  const commonUrl = resolveCommonBrowserUrl(config);
  if (commonUrl) return commonUrl;

  const baseUrl = resolveEngineBaseUrl(config);
  if (!baseUrl) return normalizeDisplayUrl(window.location.origin);
  if (baseUrl.startsWith("/")) {
    return normalizeDisplayUrl(
      new URL(baseUrl, window.location.origin).toString(),
    );
  }
  return normalizeDisplayUrl(baseUrl);
}

export const Toolbar = ({ activeTab }: ToolbarProps) => {
  const dispatch = useAppDispatch();
  const activeTabRef = useRef<HTMLDivElement | null>(null);
  const { isDarkMode, toggle: toggleDarkMode } = useAppearance();
  const config = useConfig();
  const { host } = config;
  const openUrl = useOpenUrl();
  const engineUrl = useMemo(() => resolveBrowserEngineUrl(config), [config]);

  const allThreads = useAppSelector(selectAllThreads);
  const currentChatId = useAppSelector(selectChatId);

  const openTasks = useAppSelector(selectOpenTasksFromRoot);
  const { data: tasksList = [] } = useListTasksQuery(undefined);

  const { openSettings } = useEventsBusForIDE();
  const [createTask] = useCreateTaskMutation();

  const [draggingTabId, setDraggingTabId] = useState<string | null>(null);
  const [renameState, setRenameState] = useState<{
    kind: "task";
    id: string;
    value: string;
  } | null>(null);
  const [updateTaskMeta] = useUpdateTaskMetaMutation();

  const handleNavigation = useCallback(
    (to: DropdownNavigationOptions | "chat") => {
      if (to === "settings") {
        openSettings();
      } else if (to === "general settings") {
        dispatch(push({ name: "general settings" }));
      } else if (to === "stats") {
        dispatch(push({ name: "stats dashboard" }));
      } else if (to === "knowledge graph") {
        dispatch(push({ name: "knowledge graph" }));
      } else if (to === "chat") {
        dispatch(popBackTo({ name: "history" }));
        dispatch(push({ name: "chat" }));
      }
    },
    [dispatch, openSettings],
  );

  const onCreateNewChat = useCallback(() => {
    setRenameState(null);

    const currentThread = allThreads[currentChatId] as
      | { thread: { messages: unknown[] } }
      | undefined;

    dispatch(clearThreadPauseReasons({ id: currentChatId }));
    dispatch(
      setThreadConfirmationStatus({
        id: currentChatId,
        wasInteracted: false,
        confirmationStatus: true,
      }),
    );

    if (currentThread && currentThread.thread.messages.length === 0) {
      dispatch(closeThread({ id: currentChatId }));
    }

    dispatch(newChatAction());
    handleNavigation("chat");
  }, [dispatch, currentChatId, allThreads, handleNavigation]);

  const onCreateNewTask = useCallback(() => {
    createTask({ name: "New Task" })
      .unwrap()
      .then((task) => {
        dispatch(openTask({ id: task.id, name: task.name }));
        dispatch(push({ name: "task workspace", taskId: task.id }));
      })
      .catch(() => {
        /* handled by RTK Query */
      });
  }, [createTask, dispatch]);

  const onOpenChatInBrowser = useCallback(() => {
    openUrl(engineUrl);
  }, [engineUrl, openUrl]);

  const goToTab = useCallback(
    (tab: Tab) => {
      const isSameTab =
        (isChatTab(tab) && isChatTab(activeTab) && tab.id === activeTab.id) ||
        (isTaskTab(tab) &&
          isTaskTab(activeTab) &&
          tab.taskId === activeTab.taskId);

      if (isSameTab) {
        return;
      }

      if (isChatTab(activeTab)) {
        const currentThread = allThreads[activeTab.id];
        if (currentThread && currentThread.thread.messages.length === 0) {
          dispatch(closeThread({ id: activeTab.id }));
        }
      }

      if (tab.type === "dashboard") {
        dispatch(popBackTo({ name: "history" }));
      } else if (tab.type === "task") {
        dispatch(popBackTo({ name: "history" }));
        dispatch(push({ name: "task workspace", taskId: tab.taskId }));
      } else {
        dispatch(switchToThread({ id: tab.id }));
        dispatch(popBackTo({ name: "history" }));
        dispatch(push({ name: "chat" }));
      }
    },
    [dispatch, activeTab, allThreads],
  );

  const tabByValue = useMemo(() => {
    const next = new Map<string, Tab>();
    openTasks.forEach((task) => {
      next.set(task.id, {
        type: "task",
        taskId: task.id,
        taskName: task.name.trim() || "Task",
      });
    });
    return next;
  }, [openTasks]);

  const handleTabValueChange = useCallback(
    (value: string) => {
      const nextTab = tabByValue.get(value);
      if (nextTab) {
        goToTab(nextTab);
      }
    },
    [goToTab, tabByValue],
  );

  const handleCloseTaskTab = useCallback(
    (event: MouseEvent, taskId: string) => {
      event.stopPropagation();
      event.preventDefault();
      dispatch(closeTask(taskId));
      if (isTaskTab(activeTab) && activeTab.taskId === taskId) {
        goToTab({ type: "dashboard" });
      }
    },
    [dispatch, activeTab, goToTab],
  );

  useEffect(() => {
    if (activeTabRef.current?.scrollIntoView) {
      activeTabRef.current.scrollIntoView({
        behavior: "smooth",
        block: "nearest",
        inline: "nearest",
      });
    }
  }, [activeTab]);

  const handleTaskRenaming = useCallback(
    (taskId: string, currentName: string) => {
      setRenameState({ kind: "task", id: taskId, value: currentName });
    },
    [],
  );

  const handleKeyUpOnTaskRename = useCallback(
    (event: KeyboardEvent<HTMLInputElement>, taskId: string) => {
      if (event.code === "Escape") {
        setRenameState(null);
      }
      if (event.code === "Enter") {
        const name = renameState?.value.trim();
        setRenameState(null);
        if (!name) return;
        void updateTaskMeta({ taskId, name });
      }
    },
    [renameState, updateTaskMeta],
  );

  const handleRenameChange = (value: string) => {
    setRenameState((prev) => (prev ? { ...prev, value } : null));
  };

  const handleMiddleClickClose = useCallback(
    (event: MouseEvent, tab: TaskTab) => {
      if (event.button !== 1) return;
      handleCloseTaskTab(event, tab.taskId);
    },
    [handleCloseTaskTab],
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
    (event: DragEvent, id: string) => {
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", tabDragData("task", id));
      setDraggingTabId(id);
    },
    [setDraggingTabId],
  );

  const handleDragEnd = useCallback(() => {
    setDraggingTabId(null);
  }, [setDraggingTabId]);

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
      if (!dragged || dragged.type !== "task" || dragged.id === id) return;
      dispatch(reorderOpenTasks({ sourceId: dragged.id, targetId: id }));
    },
    [dispatch],
  );

  return (
    <div className={styles.toolbar}>
      <div className={styles.toolbarSection}>
        <ToolbarIconButton
          label="Home"
          className={styles.homeButton}
          icon={Home}
          onClick={() => {
            setRenameState(null);
            goToTab({ type: "dashboard" });
          }}
        />
      </div>

      <div className={styles.toolbarDivider} />

      <KitTabs
        className={classNames(styles.tabsContainer, "scrollX")}
        onWheel={(event) => {
          const container = event.currentTarget;
          if (container.scrollWidth <= container.clientWidth) return;
          event.preventDefault();
          container.scrollLeft += event.deltaY || event.deltaX;
        }}
        value={isTaskTab(activeTab) ? activeTab.taskId : "dashboard"}
        onValueChange={handleTabValueChange}
      >
        <KitTabs.List className={styles.tabList}>
          {openTasks.map((task) => {
            const isActive =
              isTaskTab(activeTab) && activeTab.taskId === task.id;
            const taskName = task.name.trim() || "Task";
            const isRenaming =
              renameState?.kind === "task" && renameState.id === task.id;

            if (isRenaming) {
              return (
                <div key={`task-${task.id}`} className={styles.tabWrap}>
                  <FieldText
                    autoComplete="off"
                    onKeyUp={(e) => handleKeyUpOnTaskRename(e, task.id)}
                    onBlur={() => setRenameState(null)}
                    autoFocus
                    value={renameState.value}
                    onChange={handleRenameChange}
                    className={styles.RenameInput}
                  />
                </div>
              );
            }

            const taskMeta = tasksList.find((t) => t.id === task.id);
            const taskStatus = taskMeta
              ? getTaskStatusDotState(taskMeta)
              : "idle";

            return (
              <div
                key={`task-${task.id}`}
                className={classNames(
                  styles.tabWrap,
                  draggingTabId === task.id && styles.tabWrapDragging,
                )}
                onDragOver={handleDragOver}
                onDrop={(event) => handleDrop(event, task.id)}
                ref={isActive ? activeTabRef : undefined}
              >
                <KitTabs.Trigger value={task.id} asChild>
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
                      handleMiddleClickClose(event, {
                        type: "task",
                        taskId: task.id,
                        taskName,
                      })
                    }
                    onDoubleClick={() => handleTaskRenaming(task.id, taskName)}
                    onDragStart={(event) => handleDragStart(event, task.id)}
                    onDragEnd={handleDragEnd}
                    title={taskName}
                  >
                    <span className={styles.tabStatus}>
                      <StatusDot
                        aria-label={taskStatusLabel(taskStatus)}
                        status={taskStatus}
                        size="small"
                      />
                    </span>
                    <span className={styles.tabTitle}>{taskName}</span>
                  </button>
                </KitTabs.Trigger>
                <button
                  type="button"
                  className={styles.tabClose}
                  title="Close task tab"
                  aria-label="Close tab"
                  draggable={false}
                  onMouseDown={stopClosePointerEvent}
                  onPointerDown={stopClosePointerEvent}
                  onDragStart={stopCloseDragEvent}
                  onClick={(e) => handleCloseTaskTab(e, task.id)}
                >
                  <Icon icon={X} size="sm" tone="muted" />
                </button>
              </div>
            );
          })}
        </KitTabs.List>
      </KitTabs>

      <div
        className={classNames(styles.toolbarDivider, styles.connectionDivider)}
      />

      <div
        className={classNames(styles.toolbarSection, styles.connectionSection)}
      >
        <ConnectionStatusIndicator />
        <a
          className={styles.engineUrl}
          href={engineUrl}
          title={engineUrl}
          aria-label={`Engine URL ${engineUrl}`}
          onClick={(event) => {
            event.preventDefault();
            onOpenChatInBrowser();
          }}
        >
          {engineUrl}
        </a>
      </div>

      {activeTab.type !== "dashboard" && (
        <>
          <div className={styles.toolbarDivider} />

          <div
            className={classNames(styles.toolbarSection, styles.actionSection)}
          >
            <ToolbarIconButton
              label="New Chat"
              icon={Plus}
              onClick={onCreateNewChat}
            />

            <ToolbarIconButton
              label="New Task"
              icon={CheckSquare}
              className={styles.newTaskAction}
              onClick={onCreateNewTask}
            />
          </div>
        </>
      )}

      <div className={styles.toolbarDivider} />

      <div className={classNames(styles.toolbarSection, styles.menuSection)}>
        {host === "web" && (
          <ToolbarIconButton
            label="Toggle Dark Mode"
            icon={isDarkMode ? Moon : Sun}
            className={styles.themeToggleAction}
            onClick={toggleDarkMode}
          />
        )}

        <Dropdown handleNavigation={handleNavigation} useGhostTrigger />
      </div>
    </div>
  );
};

import { TextField, HoverCard, Text, Badge } from "@radix-ui/themes";
import { Dropdown, DropdownNavigationOptions } from "./Dropdown";
import {
  Cross1Icon,
  PlusIcon,
  CheckboxIcon,
  ExternalLinkIcon,
  MoonIcon,
  SunIcon,
} from "@radix-ui/react-icons";
import classNames from "classnames";
import { RefactIcon } from "../../images";
import { newChatAction } from "../../events";
import {
  getStatusFromSessionState,
  getTaskStatusDotState,
} from "../../utils/sessionStatus";
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
} from "../../features/Tasks";
import {
  ChangeEvent,
  KeyboardEvent,
  MouseEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { updateChatTitleById } from "../../features/History/historySlice";
import {
  saveTitle,
  selectAllThreads,
  selectTabsDisplayData,
  closeThread,
  switchToThread,
  selectChatId,
  clearThreadPauseReasons,
  setThreadConfirmationStatus,
} from "../../features/Chat";
import { StatusDot } from "../StatusDot";
import {
  useAppDispatch,
  useAppSelector,
  useAppearance,
  useEventsBusForIDE,
  useOpenUrl,
} from "../../hooks";
import { useGetChatModesQuery } from "../../services/refact/chatModes";

import styles from "./Toolbar.module.css";
import { ConnectionStatusIndicator } from "../ConnectionStatus";
import { getModeColor } from "../../utils/modeColors";
import { selectLspPort } from "../../features/Config/configSlice";

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
  children: React.ReactNode;
  className?: string;
  disabled?: boolean;
};

const ToolbarIconButton = ({
  label,
  onClick,
  children,
  className,
  disabled,
}: ToolbarIconButtonProps) => (
  <HoverCard.Root>
    <HoverCard.Trigger>
      <button
        type="button"
        className={classNames(styles.iconButton, className)}
        onClick={onClick}
        aria-label={label}
        disabled={disabled}
      >
        {children}
      </button>
    </HoverCard.Trigger>
    <HoverCard.Content size="1" side="bottom">
      <Text as="p" size="2">
        {label}
      </Text>
    </HoverCard.Content>
  </HoverCard.Root>
);

export const Toolbar = ({ activeTab }: ToolbarProps) => {
  const dispatch = useAppDispatch();
  const scrollContainerRef = useRef<HTMLDivElement | null>(null);
  const activeTabRef = useRef<HTMLDivElement | null>(null);
  const { isDarkMode, toggle: toggleDarkMode } = useAppearance();
  const openUrl = useOpenUrl();

  const tabs = useAppSelector(selectTabsDisplayData);
  const allThreads = useAppSelector(selectAllThreads);
  const currentChatId = useAppSelector(selectChatId);
  const lspPort = useAppSelector(selectLspPort);
  const openTasks = useAppSelector(selectOpenTasksFromRoot);
  const { data: modesData } = useGetChatModesQuery(undefined);
  const { data: tasksList = [] } = useListTasksQuery(undefined);

  const { openSettings, openHotKeys } = useEventsBusForIDE();
  const [createTask] = useCreateTaskMutation();

  const [renameState, setRenameState] = useState<{
    kind: "chat" | "task";
    id: string;
    value: string;
  } | null>(null);
  const [updateTaskMeta] = useUpdateTaskMetaMutation();

  const handleNavigation = useCallback(
    (to: DropdownNavigationOptions | "chat") => {
      if (to === "settings") {
        openSettings();
      } else if (to === "hot keys") {
        openHotKeys();
      } else if (to === "fim") {
        dispatch(push({ name: "fill in the middle debug page" }));
      } else if (to === "stats") {
        dispatch(push({ name: "stats dashboard" }));
      } else if (to === "integrations") {
        dispatch(push({ name: "integrations page" }));
      } else if (to === "providers") {
        dispatch(push({ name: "providers page" }));
      } else if (to === "knowledge graph") {
        dispatch(push({ name: "knowledge graph" }));
      } else if (to === "customization") {
        dispatch(push({ name: "customization" }));
      } else if (to === "default models") {
        dispatch(push({ name: "default models" }));
      } else if (to === "extensions") {
        dispatch(push({ name: "extensions" }));
      } else if (to === "chat") {
        dispatch(popBackTo({ name: "history" }));
        dispatch(push({ name: "chat" }));
      }
    },
    [dispatch, openSettings, openHotKeys],
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
    openUrl(`http://127.0.0.1:${lspPort}`);
  }, [lspPort, openUrl]);

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
  }, [currentChatId, activeTab]);

  const handleChatThreadRenaming = useCallback(
    (tabId: string, currentTitle: string) => {
      setRenameState({ kind: "chat", id: tabId, value: currentTitle });
    },
    [],
  );

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

  const handleRenameChange = (event: ChangeEvent<HTMLInputElement>) => {
    setRenameState((prev) =>
      prev ? { ...prev, value: event.target.value } : null,
    );
  };

  const handleCloseTab = useCallback(
    (event: MouseEvent, tabId: string) => {
      event.stopPropagation();
      event.preventDefault();
      dispatch(closeThread({ id: tabId }));
      if (activeTab.type === "chat" && activeTab.id === tabId) {
        const remainingTabs = tabs.filter((t) => t.id !== tabId);
        if (remainingTabs.length > 0) {
          goToTab({ type: "chat", id: remainingTabs[0].id });
        } else {
          goToTab({ type: "dashboard" });
        }
      }
    },
    [dispatch, activeTab, tabs, goToTab],
  );

  const handleWheel = useCallback((event: React.WheelEvent<HTMLDivElement>) => {
    const container = scrollContainerRef.current;
    if (!container) return;
    if (container.scrollWidth <= container.clientWidth) return;
    event.preventDefault();
    container.scrollLeft += event.deltaY || event.deltaX;
  }, []);

  return (
    <div className={styles.toolbar}>
      <div className={styles.toolbarSection}>
        <ToolbarIconButton
          label="Home"
          className={styles.homeButton}
          onClick={() => {
            setRenameState(null);
            goToTab({ type: "dashboard" });
          }}
        >
          <RefactIcon style={{ color: "#E7150D" }} />
        </ToolbarIconButton>
      </div>

      <div className={styles.toolbarDivider} />

      <div
        ref={scrollContainerRef}
        className={styles.tabsContainer}
        onWheel={handleWheel}
      >
        <div role="tablist" className={styles.tabList}>
          {openTasks.map((task) => {
            const isActive =
              isTaskTab(activeTab) && activeTab.taskId === task.id;
            const taskName = task.name.trim() || "Task";
            const isRenaming =
              renameState?.kind === "task" && renameState.id === task.id;

            if (isRenaming) {
              return (
                <div key={`task-${task.id}`} className={styles.tabWrap}>
                  <TextField.Root
                    autoComplete="off"
                    onKeyUp={(e) => handleKeyUpOnTaskRename(e, task.id)}
                    onBlur={() => setRenameState(null)}
                    autoFocus
                    size="1"
                    value={renameState.value}
                    onChange={handleRenameChange}
                    className={styles.RenameInput}
                  />
                </div>
              );
            }

            const taskMeta = tasksList.find((t) => t.id === task.id);

            return (
              <div
                key={`task-${task.id}`}
                className={styles.tabWrap}
                ref={isActive ? activeTabRef : undefined}
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={isActive}
                  className={`${styles.tabButton} ${
                    isActive ? styles.tabButtonActive : ""
                  }`}
                  onClick={() =>
                    goToTab({ type: "task", taskId: task.id, taskName })
                  }
                  onDoubleClick={() => handleTaskRenaming(task.id, taskName)}
                  title={taskName}
                >
                  <span className={styles.tabStatus}>
                    <StatusDot
                      state={
                        taskMeta ? getTaskStatusDotState(taskMeta) : "idle"
                      }
                      size="small"
                    />
                  </span>
                  <span className={styles.tabTitle}>{taskName}</span>
                </button>
                <button
                  type="button"
                  className={styles.tabClose}
                  title="Close task tab"
                  onClick={(e) => handleCloseTaskTab(e, task.id)}
                >
                  <Cross1Icon width={10} height={10} />
                </button>
              </div>
            );
          })}

          {tabs.map((tab) => {
            const isActive = isChatTab(activeTab) && activeTab.id === tab.id;
            const isRenaming =
              renameState?.kind === "chat" && renameState.id === tab.id;

            if (isRenaming) {
              return (
                <div key={tab.id} className={styles.tabWrap}>
                  <TextField.Root
                    autoComplete="off"
                    onKeyUp={(e) => handleKeyUpOnRename(e, tab.id)}
                    onBlur={() => setRenameState(null)}
                    autoFocus
                    size="1"
                    value={renameState.value}
                    onChange={handleRenameChange}
                    className={styles.RenameInput}
                  />
                </div>
              );
            }

            const statusState = getStatusFromSessionState(tab.session_state);

            const modeInfo = modesData?.modes.find((m) => m.id === tab.mode);
            const modeLabel = modeInfo?.title ?? tab.mode;

            return (
              <div
                key={tab.id}
                className={styles.tabWrap}
                ref={isActive ? activeTabRef : undefined}
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={isActive}
                  className={`${styles.tabButton} ${
                    isActive ? styles.tabButtonActive : ""
                  }`}
                  onClick={() => goToTab({ type: "chat", id: tab.id })}
                  onDoubleClick={() =>
                    handleChatThreadRenaming(tab.id, tab.title)
                  }
                  title={tab.title}
                >
                  <span className={styles.tabStatus}>
                    <StatusDot state={statusState} size="small" />
                  </span>
                  <span className={styles.tabTitle}>{tab.title}</span>
                  {!tab.is_buddy_chat && modeLabel && (
                    <Badge
                      size="1"
                      color={getModeColor(tab.mode)}
                      variant="soft"
                      className={styles.tabModeBadge}
                    >
                      {modeLabel}
                    </Badge>
                  )}
                </button>
                <button
                  type="button"
                  className={styles.tabClose}
                  title="Close tab"
                  onClick={(e) => handleCloseTab(e, tab.id)}
                >
                  <Cross1Icon width={10} height={10} />
                </button>
              </div>
            );
          })}
        </div>
      </div>

      <div className={styles.toolbarDivider} />

      <div className={styles.toolbarSection}>
        <ConnectionStatusIndicator />
      </div>

      <div className={styles.toolbarDivider} />

      <div className={styles.toolbarSection}>
        <ToolbarIconButton label="New Chat" onClick={onCreateNewChat}>
          <PlusIcon />
        </ToolbarIconButton>

        <ToolbarIconButton label="New Task" onClick={onCreateNewTask}>
          <CheckboxIcon />
        </ToolbarIconButton>
      </div>

      <div className={styles.toolbarDivider} />

      <div className={styles.toolbarSection}>
        <ToolbarIconButton
          label="Open Chat in Browser"
          onClick={onOpenChatInBrowser}
        >
          <ExternalLinkIcon />
        </ToolbarIconButton>

        <ToolbarIconButton label="Toggle Dark Mode" onClick={toggleDarkMode}>
          {isDarkMode ? <MoonIcon /> : <SunIcon />}
        </ToolbarIconButton>

        <Dropdown handleNavigation={handleNavigation} useGhostTrigger />
      </div>
    </div>
  );
};

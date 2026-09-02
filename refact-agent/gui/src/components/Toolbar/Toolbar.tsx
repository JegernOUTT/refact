import { Dropdown, DropdownNavigationOptions } from "./Dropdown";
import { Home } from "lucide-react";
import classNames from "classnames";
import { ComponentProps, useCallback } from "react";

import { newChatAction } from "../../events";
import {
  clearThreadPauseReasons,
  closeThread,
  selectAllThreads,
  selectChatId,
  setThreadConfirmationStatus,
  switchToThread,
} from "../../features/Chat/Thread";
import { popBackTo, push, selectPages } from "../../features/Pages/pagesSlice";
import { openTask, selectOpenTasksFromRoot } from "../../features/Tasks";
import { selectCapabilities } from "../../features/Config/configSlice";
import {
  selectFocusedWorkspaceChatId,
  selectPanelsForced,
  selectTabs,
} from "../../features/Workspace";
import { TabBar } from "../../features/Workspace/TabBar";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useConfig } from "../../hooks/useConfig";
import { useEventsBusForIDE } from "../../hooks/useEventBusForIDE";
import { useCreateTaskMutation } from "../../services/refact/tasks";
import { resolveWorkspaceDockAvailability } from "../../features/Workspace/workspaceAvailability";
import { IconButton, Tooltip } from "../ui";
import { EngineStatusChip } from "../ConnectionStatus";
import { NewSplitButton } from "./NewSplitButton";
import styles from "./Toolbar.module.css";

export type DashboardTab = {
  type: "dashboard";
};

export type ChatTab = {
  type: "chat";
  id: string;
};

export type TaskTab = {
  type: "task";
  taskId: string;
  taskName: string;
};

export type BuddyTab = {
  type: "buddy";
};

export type Tab = DashboardTab | ChatTab | TaskTab | BuddyTab;

export type ToolbarProps = {
  activeTab: Tab;
};

type ToolbarIconButtonProps = {
  label: string;
  onClick: () => void;
  icon: ComponentProps<typeof IconButton>["icon"];
  className?: string;
};

const ToolbarIconButton = ({
  label,
  onClick,
  icon,
  className,
}: ToolbarIconButtonProps) => (
  <Tooltip>
    <Tooltip.Trigger asChild>
      <IconButton
        aria-label={label}
        className={classNames(styles.iconButton, "rf-pressable", className)}
        icon={icon}
        onClick={onClick}
        size="sm"
        variant="plain"
      />
    </Tooltip.Trigger>
    <Tooltip.Content side="bottom">{label}</Tooltip.Content>
  </Tooltip>
);

export const Toolbar = ({ activeTab }: ToolbarProps) => {
  const dispatch = useAppDispatch();
  const { host } = useConfig();
  const allThreads = useAppSelector(selectAllThreads);
  const currentChatId = useAppSelector(selectChatId);
  const focusedWorkspaceChatId = useAppSelector(selectFocusedWorkspaceChatId);
  const workspaceTabs = useAppSelector(selectTabs);
  const openTasks = useAppSelector(selectOpenTasksFromRoot);
  const pages = useAppSelector(selectPages);
  const capabilities = useAppSelector(selectCapabilities);
  const panelsForced = useAppSelector(selectPanelsForced);
  const workspaceAvailability = resolveWorkspaceDockAvailability(
    host,
    capabilities,
    panelsForced,
  );
  const { openSettings } = useEventsBusForIDE();
  const toolbarChatId =
    activeTab.type === "chat"
      ? activeTab.id
      : focusedWorkspaceChatId ?? currentChatId;
  const shouldCleanToolbarChat =
    activeTab.type === "chat" || focusedWorkspaceChatId !== null;
  const showTabBar =
    workspaceTabs.length > 0 ||
    openTasks.length > 0 ||
    pages.some((page) => page.name === "buddy") ||
    workspaceAvailability.dock;
  const [createTask] = useCreateTaskMutation();

  const goHome = useCallback(() => {
    if (activeTab.type === "chat") {
      const currentThread = allThreads[activeTab.id];
      if (currentThread && currentThread.thread.messages.length === 0) {
        dispatch(closeThread({ id: activeTab.id }));
      }
    }

    dispatch(popBackTo({ name: "history" }));
  }, [activeTab, allThreads, dispatch]);

  const handleNavigation = useCallback(
    (to: DropdownNavigationOptions | "chat") => {
      if (to === "settings") {
        openSettings();
      } else if (to === "general settings") {
        dispatch(push({ name: "general settings" }));
      } else if (to === "stats") {
        dispatch(push({ name: "stats dashboard" }));
      } else if (to === "performance") {
        dispatch(push({ name: "performance" }));
      } else if (to === "knowledge graph") {
        dispatch(push({ name: "knowledge graph" }));
      } else if (to === "code intel") {
        dispatch(push({ name: "code intel" }));
      } else if (to === "bug report") {
        dispatch(push({ name: "bug report" }));
      } else if (to === "chat") {
        dispatch(popBackTo({ name: "history" }));
        dispatch(push({ name: "chat" }));
      }
    },
    [dispatch, openSettings],
  );

  const onCreateNewChat = useCallback(() => {
    const currentThread = shouldCleanToolbarChat
      ? (allThreads[toolbarChatId] as
          | { thread: { messages: unknown[] } }
          | undefined)
      : undefined;

    if (currentThread && toolbarChatId !== currentChatId) {
      dispatch(switchToThread({ id: toolbarChatId, openTab: false }));
    }

    if (shouldCleanToolbarChat) {
      dispatch(clearThreadPauseReasons({ id: toolbarChatId }));
      dispatch(
        setThreadConfirmationStatus({
          id: toolbarChatId,
          wasInteracted: false,
          confirmationStatus: true,
        }),
      );
    }

    if (currentThread && currentThread.thread.messages.length === 0) {
      dispatch(closeThread({ id: toolbarChatId }));
    }

    dispatch(newChatAction());
    handleNavigation("chat");
  }, [
    allThreads,
    currentChatId,
    shouldCleanToolbarChat,
    toolbarChatId,
    dispatch,
    handleNavigation,
  ]);

  const onCreateNewTask = useCallback(() => {
    void createTask({ name: "New Task" })
      .unwrap()
      .then((task) => {
        dispatch(openTask({ id: task.id, name: task.name }));
        dispatch(push({ name: "task workspace", taskId: task.id }));
      })
      .catch(() => undefined);
  }, [createTask, dispatch]);

  return (
    <div className={styles.toolbar}>
      <div className={styles.toolbarSection}>
        <ToolbarIconButton
          label="Home"
          className={styles.homeButton}
          icon={Home}
          onClick={goHome}
        />
      </div>

      {showTabBar ? (
        <>
          <div className={styles.toolbarDivider} />
          <TabBar placement="toolbar" />
        </>
      ) : (
        <div className={styles.toolbarSpacer} data-element="ToolbarSpacer" />
      )}

      <div
        className={classNames(styles.toolbarDivider, styles.connectionDivider)}
      />

      <div
        className={classNames(styles.toolbarSection, styles.connectionSection)}
      >
        <EngineStatusChip />
      </div>

      <div className={styles.toolbarDivider} />

      <div className={classNames(styles.toolbarSection, styles.actionSection)}>
        <NewSplitButton
          onCreateNewChat={onCreateNewChat}
          onCreateNewTask={onCreateNewTask}
        />
      </div>

      <div className={styles.toolbarDivider} />

      <div className={classNames(styles.toolbarSection, styles.menuSection)}>
        <Dropdown handleNavigation={handleNavigation} />
      </div>
    </div>
  );
};

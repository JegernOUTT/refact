import { Dropdown, DropdownNavigationOptions } from "./Dropdown";
import { CheckSquare, Home, Moon, Plus, Sun } from "lucide-react";
import classNames from "classnames";
import { ComponentProps, useCallback, useMemo } from "react";

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
import { selectTabs } from "../../features/Workspace";
import { TabBar } from "../../features/Workspace/TabBar";
import {
  useAppDispatch,
  useAppSelector,
  useAppearance,
  useConfig,
  useEventsBusForIDE,
  useOpenUrl,
} from "../../hooks";
import { useCreateTaskMutation } from "../../services/refact/tasks";
import {
  resolveEngineBaseUrl,
  type EngineApiConfig,
} from "../../services/refact/apiUrl";
import { IconButton, Tooltip } from "../ui";
import { ConnectionStatusIndicator } from "../ConnectionStatus";
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
  const { isDarkMode, toggle: toggleDarkMode } = useAppearance();
  const config = useConfig();
  const { host } = config;
  const openUrl = useOpenUrl();
  const engineUrl = useMemo(() => resolveBrowserEngineUrl(config), [config]);
  const allThreads = useAppSelector(selectAllThreads);
  const currentChatId = useAppSelector(selectChatId);
  const workspaceTabs = useAppSelector(selectTabs);
  const openTasks = useAppSelector(selectOpenTasksFromRoot);
  const pages = useAppSelector(selectPages);
  const { openSettings } = useEventsBusForIDE();
  const toolbarChatId =
    activeTab.type === "chat" ? activeTab.id : currentChatId;
  const showTabBar =
    workspaceTabs.length > 0 ||
    openTasks.length > 0 ||
    pages.some((page) => page.name === "buddy");
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
    const currentThread = allThreads[toolbarChatId] as
      | { thread: { messages: unknown[] } }
      | undefined;

    if (currentThread && toolbarChatId !== currentChatId) {
      dispatch(switchToThread({ id: toolbarChatId, openTab: false }));
    }

    dispatch(clearThreadPauseReasons({ id: toolbarChatId }));
    dispatch(
      setThreadConfirmationStatus({
        id: toolbarChatId,
        wasInteracted: false,
        confirmationStatus: true,
      }),
    );

    if (currentThread && currentThread.thread.messages.length === 0) {
      dispatch(closeThread({ id: toolbarChatId }));
    }

    dispatch(newChatAction());
    handleNavigation("chat");
  }, [allThreads, currentChatId, toolbarChatId, dispatch, handleNavigation]);

  const onCreateNewTask = useCallback(() => {
    void createTask({ name: "New Task" })
      .unwrap()
      .then((task) => {
        dispatch(openTask({ id: task.id, name: task.name }));
        dispatch(push({ name: "task workspace", taskId: task.id }));
      })
      .catch(() => undefined);
  }, [createTask, dispatch]);

  const onOpenChatInBrowser = useCallback(() => {
    openUrl(engineUrl);
  }, [engineUrl, openUrl]);

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

      {showTabBar && (
        <>
          <div className={styles.toolbarDivider} />
          <TabBar placement="toolbar" />
        </>
      )}

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

      <div className={styles.toolbarDivider} />

      <div className={classNames(styles.toolbarSection, styles.actionSection)}>
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

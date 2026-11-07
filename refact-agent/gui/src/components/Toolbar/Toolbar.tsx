import { Dropdown, DropdownNavigationOptions } from "./Dropdown";
import classNames from "classnames";
import { useCallback, useLayoutEffect, useRef, useState } from "react";

import { newChatAction } from "../../events";
import {
  clearThreadPauseReasons,
  closeThread,
  selectAllThreads,
  selectChatId,
  setThreadConfirmationStatus,
  switchToThread,
} from "../../features/Chat/Thread";
import { popBackTo, push } from "../../features/Pages/pagesSlice";
import { openTask } from "../../features/Tasks";
import {
  selectTabs,
  selectWorkspaceGroups,
  selectWorkspaceFocusedChatId,
} from "../../features/Workspace/workspaceSlice";
import { collectTabIds } from "../../features/ChatPanes/panesTree";
import { isFileSurface } from "../../features/Workspace/surfaceKey";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useEventsBusForIDE } from "../../hooks/useEventBusForIDE";
import { useCreateTaskMutation } from "../../services/refact/tasks";
import { EngineStatusChip } from "../ConnectionStatus";
import { NewSplitButton } from "./NewSplitButton";
import { WorkspaceToolbarTabs } from "../../features/Workspace/WorkspacePaneTabs";
import {
  needsCompactEngine,
  type PaneTabFitChange,
} from "../../features/Workspace/paneTabFit";
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

export const Toolbar = ({ activeTab }: ToolbarProps) => {
  const toolbarRef = useRef<HTMLDivElement>(null);
  const tabsRef = useRef<HTMLDivElement>(null);
  const stripWidths = useRef(new Map<HTMLElement, number>());
  const scheduleFit = useRef<() => void>();
  const [compactEngine, setCompactEngine] = useState(false);
  const compactRef = useRef(compactEngine);
  compactRef.current = compactEngine;
  const onFitChange = useCallback<PaneTabFitChange>((element, width) => {
    if (width === null) stripWidths.current.delete(element);
    else stripWidths.current.set(element, width);
    scheduleFit.current?.();
  }, []);
  useLayoutEffect(() => {
    let frame = 0;
    const measure = () => {
      frame = 0;
      const slot = tabsRef.current;
      const label = toolbarRef.current?.querySelector<HTMLElement>(
        "[data-engine-label]",
      );
      if (!slot || !label) return;
      const chip = label.parentElement;
      const savings =
        label.getBoundingClientRect().width +
        (chip ? parseFloat(getComputedStyle(chip).columnGap) || 0 : 0);
      const expandedRight =
        slot.getBoundingClientRect().right - (compactRef.current ? savings : 0);
      const strips = Array.from(
        stripWidths.current,
        ([element, naturalWidth]) => ({
          left: element.getBoundingClientRect().left,
          right: element.getBoundingClientRect().right,
          naturalWidth,
        }),
      );
      setCompactEngine(needsCompactEngine(strips, expandedRight));
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(measure);
    };
    scheduleFit.current = schedule;
    const observer = new ResizeObserver(schedule);
    if (toolbarRef.current) observer.observe(toolbarRef.current);
    if (tabsRef.current) observer.observe(tabsRef.current);
    const label = toolbarRef.current?.querySelector<HTMLElement>(
      "[data-engine-label]",
    );
    if (label) observer.observe(label);
    schedule();
    return () => {
      scheduleFit.current = undefined;
      observer.disconnect();
      cancelAnimationFrame(frame);
    };
  }, []);
  const dispatch = useAppDispatch();
  const allThreads = useAppSelector(selectAllThreads);
  const currentChatId = useAppSelector(selectChatId);
  const focusedWorkspaceChatId = useAppSelector(selectWorkspaceFocusedChatId);
  const workspaceTabs = useAppSelector(selectTabs);
  const workspaceGroups = useAppSelector(selectWorkspaceGroups);
  const hasOpenFiles =
    workspaceTabs.some(isFileSurface) ||
    Object.values(workspaceGroups).some((group) =>
      group ? collectTabIds(group.root).some(isFileSurface) : false,
    );
  const { openSettings } = useEventsBusForIDE();
  const toolbarChatId =
    activeTab.type === "chat"
      ? activeTab.id
      : focusedWorkspaceChatId ?? currentChatId;
  const shouldCleanToolbarChat =
    activeTab.type === "chat" || focusedWorkspaceChatId !== null;
  const [createTask] = useCreateTaskMutation();

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

    if (
      currentThread &&
      currentThread.thread.messages.length === 0 &&
      !hasOpenFiles
    ) {
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
    hasOpenFiles,
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
    <div
      ref={toolbarRef}
      className={styles.toolbar}
      role="toolbar"
      aria-label="Workspace toolbar"
    >
      <div ref={tabsRef} className={styles.toolbarTabs}>
        <WorkspaceToolbarTabs onFitChange={onFitChange} />
      </div>

      <div
        className={classNames(styles.toolbarDivider, styles.connectionDivider)}
      />

      <div
        className={classNames(styles.toolbarSection, styles.connectionSection)}
      >
        <EngineStatusChip compact={compactEngine} />
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

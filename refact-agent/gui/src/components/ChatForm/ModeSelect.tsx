import React, {
  useRef,
  useEffect,
  useState,
  useCallback,
  useMemo,
} from "react";
import { Bot, Plus } from "lucide-react";
import {
  useGetChatModesQuery,
  ChatModeInfo,
  ChatModeThreadDefaults,
} from "../../services/refact/chatModes";
import { DEFAULT_MODE } from "../../features/Chat/Thread/types";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { selectMessagesById, useThreadId } from "../../features/Chat/Thread";
import { push, selectCurrentPage } from "../../features/Pages/pagesSlice";
import { Icon, Popover, Skeleton } from "../ui";
import {
  ComposerTrigger,
  KitBadge,
  KitChevron,
  KitDivider,
  KitRow,
  kit,
} from "../ComposerKit";
import { ModeTransitionDialog } from "./ModeTransitionDialog";
import { TaskPlannerDialog } from "./TaskPlannerDialog";
import styles from "./ModeSelect.module.css";

const TASK_PLANNER_SYNTHETIC: ChatModeInfo = {
  id: "task_planner",
  title: "Task Planner",
  description: "Create a new task and manage it with structured planning",
  tools_count: 0,
  ui: { order: 999, tags: ["tasks", "planning"] },
  thread_defaults: {
    include_project_info: false,
    checkpoints_enabled: false,
    auto_approve_editing_tools: false,
    auto_approve_dangerous_commands: false,
  },
};

type ModeSelectProps = {
  selectedMode: string;
  onModeChange: (
    modeId: string,
    threadDefaults?: ChatModeThreadDefaults,
  ) => void;
  disabled?: boolean;
  onOpenChange?: (open: boolean) => void;
};

export const ModeSelect: React.FC<ModeSelectProps> = ({
  selectedMode,
  onModeChange,
  disabled,
  onOpenChange,
}) => {
  const dispatch = useAppDispatch();
  const { data, isLoading } = useGetChatModesQuery(undefined);
  const currentChatId = useThreadId();
  const messages = useAppSelector((state) =>
    selectMessagesById(state, currentChatId),
  );
  const currentPage = useAppSelector(selectCurrentPage);

  const taskId =
    currentPage?.name === "task workspace" ? currentPage.taskId : undefined;

  const rawModes = useMemo(() => data?.modes ?? [], [data]);

  const effectiveModes = useMemo(() => {
    const hasTp = rawModes.some((m) => m.id === "task_planner");
    const withTp = hasTp ? rawModes : [...rawModes, TASK_PLANNER_SYNTHETIC];
    if (taskId) {
      return [
        ...withTp.filter((m) => m.id === "task_planner"),
        ...withTp.filter((m) => m.id !== "task_planner"),
      ];
    }
    return withTp;
  }, [rawModes, taskId]);

  const effectiveMode = (selectedMode || DEFAULT_MODE).toLowerCase();
  const currentMode = effectiveModes.find((m) => m.id === effectiveMode);
  const currentTitle = currentMode?.title ?? effectiveMode;
  const toolsCount = currentMode?.tools_count ?? 0;

  const hasMessages = messages.length > 0;
  const isModeDisabled = disabled ?? false;

  const [isOpen, setIsOpen] = useState(false);
  const handlePopoverOpenChange = useCallback(
    (open: boolean) => {
      setIsOpen(open);
      onOpenChange?.(open);
    },
    [onOpenChange],
  );
  const [transitionDialogOpen, setTransitionDialogOpen] = useState(false);
  const [targetModeForTransition, setTargetModeForTransition] =
    useState<ChatModeInfo | null>(null);
  const [taskPlannerDialogOpen, setTaskPlannerDialogOpen] = useState(false);
  const selectedModeRef = useRef<HTMLButtonElement>(null);
  const modeListRef = useRef<HTMLDivElement>(null);

  const handleModeSelect = useCallback(
    (mode: ChatModeInfo) => {
      handlePopoverOpenChange(false);
      if (mode.id === "task_planner") {
        if (taskId) {
          setTaskPlannerDialogOpen(true);
        } else if (hasMessages) {
          setTaskPlannerDialogOpen(true);
        } else {
          onModeChange(mode.id, mode.thread_defaults);
        }
        return;
      }
      if (hasMessages) {
        setTargetModeForTransition(mode);
        setTransitionDialogOpen(true);
      } else {
        onModeChange(mode.id, mode.thread_defaults);
      }
    },
    [taskId, hasMessages, onModeChange, handlePopoverOpenChange],
  );

  const handleTransitionDialogClose = useCallback((open: boolean) => {
    setTransitionDialogOpen(open);
    if (!open) {
      setTargetModeForTransition(null);
    }
  }, []);

  useEffect(() => {
    if (!isOpen) return;

    const scrollToSelected = () => {
      const container = modeListRef.current;
      const selected = selectedModeRef.current;
      if (container && selected && container.clientHeight > 0) {
        const containerHeight = container.clientHeight;
        const selectedTop = selected.offsetTop;
        const selectedHeight = selected.offsetHeight;
        container.scrollTop =
          selectedTop - containerHeight / 2 + selectedHeight / 2;
        return true;
      }
      return false;
    };

    let attempts = 0;
    const maxAttempts = 10;
    const tryScroll = () => {
      if (scrollToSelected() || attempts >= maxAttempts) return;
      attempts++;
      requestAnimationFrame(tryScroll);
    };

    requestAnimationFrame(tryScroll);
  }, [isOpen]);

  const handleCreateNewMode = () => {
    dispatch(push({ name: "customization", kind: "modes" }));
    handlePopoverOpenChange(false);
  };

  if (isLoading) {
    return (
      <Skeleton className={styles.triggerSkeleton} radius="control">
        Loading...
      </Skeleton>
    );
  }

  if (effectiveModes.length === 0) {
    return <ComposerTrigger disabled label="No modes" />;
  }

  const triggerContent = (
    <span className={styles.triggerContent}>
      <span data-show-at="2" aria-hidden="true">
        <Icon icon={Bot} size="sm" tone="muted" />
      </span>
      <span className={styles.triggerTitle} data-shrink-at="2">
        {currentTitle}
      </span>
      {toolsCount > 0 && (
        <span
          className={styles.triggerMeta}
          aria-hidden="true"
          data-shrink-at="1"
        >
          ·
        </span>
      )}
      {toolsCount > 0 && (
        <span className={styles.triggerMeta} data-shrink-at="1">
          {toolsCount} tools
        </span>
      )}
    </span>
  );

  return (
    <>
      <Popover open={isOpen} onOpenChange={handlePopoverOpenChange}>
        <Popover.Trigger asChild>
          <ComposerTrigger
            chevron
            disabled={isModeDisabled}
            aria-label={`Mode: ${currentTitle}`}
            title={
              hasMessages
                ? "Click to switch mode (context will be preserved)"
                : undefined
            }
          >
            {triggerContent}
          </ComposerTrigger>
        </Popover.Trigger>

        <Popover.Content
          className={kit.popover}
          side="top"
          align="start"
          sideOffset={8}
          maxWidth="min(340px, calc(100vw - var(--rf-space-4)))"
          maxHeight="min(420px, calc(100dvh - var(--rf-space-5)))"
          scrollable={false}
        >
          <div className={styles.modeList} ref={modeListRef}>
            {effectiveModes.map((mode) => {
              const isSelected = effectiveMode === mode.id;
              return (
                <ModeMenuItem
                  key={mode.id}
                  ref={isSelected ? selectedModeRef : undefined}
                  mode={mode}
                  isSelected={isSelected}
                  onSelect={() => handleModeSelect(mode)}
                  showTransitionHint={hasMessages}
                  isSelfSwitch={hasMessages && isSelected}
                />
              );
            })}
          </div>
          <KitDivider />
          <KitRow
            icon={<Plus aria-hidden="true" />}
            label="Create mode"
            trailing={<KitChevron />}
            onClick={handleCreateNewMode}
          />
        </Popover.Content>
      </Popover>

      {targetModeForTransition && currentChatId && (
        <ModeTransitionDialog
          open={transitionDialogOpen}
          onOpenChange={handleTransitionDialogClose}
          chatId={currentChatId}
          currentMode={effectiveMode}
          currentModeTitle={currentTitle}
          targetMode={targetModeForTransition.id}
          targetModeTitle={targetModeForTransition.title}
          targetModeDescription={targetModeForTransition.description}
        />
      )}

      <TaskPlannerDialog
        sourceChatId={currentChatId}
        open={taskPlannerDialogOpen}
        onOpenChange={setTaskPlannerDialogOpen}
        taskId={taskId}
        targetModeDescription={
          effectiveModes.find((m) => m.id === "task_planner")?.description ?? ""
        }
      />
    </>
  );
};

type ModeMenuItemProps = {
  mode: ChatModeInfo;
  isSelected: boolean;
  onSelect: () => void;
  disabled?: boolean;
  showTransitionHint?: boolean;
  isSelfSwitch?: boolean;
};

export const ModeMenuItem = React.forwardRef<
  HTMLButtonElement,
  ModeMenuItemProps
>(
  (
    { mode, isSelected, onSelect, disabled, showTransitionHint, isSelfSwitch },
    ref,
  ) => {
    const tags = mode.ui.tags.slice(0, 2).join(" · ");
    return (
      <KitRow
        ref={ref}
        label={mode.title}
        strong
        sub={mode.description || undefined}
        hint={tags || undefined}
        meta={mode.tools_count > 0 ? `${mode.tools_count} tools` : undefined}
        selected={isSelected}
        disabled={disabled}
        onClick={onSelect}
        badges={
          showTransitionHint ? (
            <KitBadge tone={isSelfSwitch ? "success" : "warning"}>
              {isSelfSwitch ? "restart" : "switch"}
            </KitBadge>
          ) : null
        }
      />
    );
  },
);

ModeMenuItem.displayName = "ModeMenuItem";
ModeSelect.displayName = "ModeSelect";

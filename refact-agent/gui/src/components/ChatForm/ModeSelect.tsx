import React, {
  useRef,
  useEffect,
  useState,
  useCallback,
  useMemo,
} from "react";
import classNames from "classnames";
import { ChevronDown, Plus, RefreshCcw, Shuffle, Wrench } from "lucide-react";
import {
  useGetChatModesQuery,
  ChatModeInfo,
  ChatModeThreadDefaults,
} from "../../services/refact/chatModes";
import { DEFAULT_MODE } from "../../features/Chat/Thread/types";
import { useAppSelector, useAppDispatch } from "../../hooks";
import {
  selectMessages,
  selectCurrentThreadId,
} from "../../features/Chat/Thread";
import { push, selectCurrentPage } from "../../features/Pages/pagesSlice";
import { Badge, Chip, Icon, Popover, Skeleton } from "../ui";
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
  const messages = useAppSelector(selectMessages);
  const currentChatId = useAppSelector(selectCurrentThreadId);
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
    return (
      <div className={classNames(styles.trigger, styles.disabled)}>
        <span className={styles.triggerTitle}>No modes</span>
      </div>
    );
  }

  const triggerContent = (
    <span className={styles.triggerContent}>
      <span className={styles.triggerTitle}>{currentTitle}</span>
      {toolsCount > 0 && (
        <span className={styles.triggerMeta} aria-hidden="true">
          ·
        </span>
      )}
      {toolsCount > 0 && (
        <span className={styles.triggerMeta}>{toolsCount} tools</span>
      )}
      <Icon icon={ChevronDown} size="sm" tone="muted" className={styles.chevron} />
    </span>
  );

  return (
    <>
      <Popover open={isOpen} onOpenChange={handlePopoverOpenChange}>
        <Popover.Trigger asChild>
          <button
            className={classNames(
              styles.trigger,
              isModeDisabled && styles.disabled,
            )}
            disabled={isModeDisabled}
            type="button"
            title={
              hasMessages
                ? "Click to switch mode (context will be preserved)"
                : undefined
            }
          >
            {triggerContent}
          </button>
        </Popover.Trigger>

        <Popover.Content
          className={styles.content}
          side="top"
          align="start"
          sideOffset={8}
          maxWidth="360px"
          maxHeight="min(420px, calc(100dvh - var(--rf-space-5)))"
        >
          <div className={styles.modeList} ref={modeListRef}>
            {effectiveModes.map((mode, index) => {
              const isSelected = effectiveMode === mode.id;
              return (
                <React.Fragment key={mode.id}>
                  {index > 0 && <div className={styles.separator} />}
                  <ModeMenuItem
                    ref={isSelected ? selectedModeRef : undefined}
                    mode={mode}
                    isSelected={isSelected}
                    onSelect={() => handleModeSelect(mode)}
                    disabled={false}
                    showTransitionHint={hasMessages}
                    isSelfSwitch={hasMessages && isSelected}
                  />
                </React.Fragment>
              );
            })}
            <div className={styles.separator} />
            <button
              className={styles.addModeItem}
              onClick={handleCreateNewMode}
              type="button"
            >
              <Icon icon={Plus} size="sm" tone="accent" />
              <span>Create new mode...</span>
            </button>
          </div>
        </Popover.Content>
      </Popover>

      {targetModeForTransition && currentChatId && (
        <ModeTransitionDialog
          open={transitionDialogOpen}
          onOpenChange={handleTransitionDialogClose}
          chatId={currentChatId}
          currentMode={effectiveMode}
          targetMode={targetModeForTransition.id}
          targetModeTitle={targetModeForTransition.title}
          targetModeDescription={targetModeForTransition.description}
        />
      )}

      <TaskPlannerDialog
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

const ModeMenuItem = React.forwardRef<HTMLButtonElement, ModeMenuItemProps>(
  (
    { mode, isSelected, onSelect, disabled, showTransitionHint, isSelfSwitch },
    ref,
  ) => {
    return (
      <button
        ref={ref}
        className={classNames(
          styles.item,
          isSelected && styles.itemSelected,
          disabled && styles.itemDisabled,
        )}
        onClick={onSelect}
        type="button"
        disabled={disabled}
      >
        <span className={styles.itemHeader}>
          <span className={styles.itemTitle}>{mode.title}</span>
          {showTransitionHint && (
            <Badge tone={isSelfSwitch ? "success" : "warning"}>
              <span className={styles.badgeContent}>
                <Icon
                  icon={isSelfSwitch ? RefreshCcw : Shuffle}
                  size="sm"
                  tone={isSelfSwitch ? "success" : "warning"}
                />
                {isSelfSwitch ? "restart" : "switch"}
              </span>
            </Badge>
          )}
        </span>

        {mode.description && (
          <span className={styles.description}>
            {mode.description.length > 80
              ? `${mode.description.slice(0, 80)}...`
              : mode.description}
          </span>
        )}

        <span className={styles.metaRow}>
          {mode.ui.tags.slice(0, 2).map((tag) => (
            <Chip key={tag} radius="chip" className={styles.chip}>
              {tag}
            </Chip>
          ))}
          {mode.tools_count > 0 && (
            <Chip
              radius="chip"
              selected={isSelected}
              className={styles.chip}
              icon={<Icon icon={Wrench} size="sm" tone="accent" />}
            >
              {mode.tools_count} tools
            </Chip>
          )}
        </span>
      </button>
    );
  },
);

ModeMenuItem.displayName = "ModeMenuItem";
ModeSelect.displayName = "ModeSelect";

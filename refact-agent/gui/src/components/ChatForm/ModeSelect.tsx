import React, {
  useRef,
  useEffect,
  useState,
  useCallback,
  useMemo,
} from "react";
import {
  Flex,
  Text,
  Badge,
  Skeleton,
  Popover,
  Separator,
} from "@radix-ui/themes";
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
};

export const ModeSelect: React.FC<ModeSelectProps> = ({
  selectedMode,
  onModeChange,
  disabled,
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

  // Thread state can carry the mode id in either case (e.g. "TASK_PLANNER" from
  // task chats vs "task_planner" from the chat-modes API). Normalise to lowercase
  // for menu-item comparisons.
  const effectiveMode = (selectedMode || DEFAULT_MODE).toLowerCase();
  const currentMode = effectiveModes.find((m) => m.id === effectiveMode);
  const currentTitle = currentMode?.title ?? effectiveMode;
  const toolsCount = currentMode?.tools_count ?? 0;

  const hasMessages = messages.length > 0;
  const isModeDisabled = disabled ?? false;

  const [isOpen, setIsOpen] = useState(false);
  const [transitionDialogOpen, setTransitionDialogOpen] = useState(false);
  const [targetModeForTransition, setTargetModeForTransition] =
    useState<ChatModeInfo | null>(null);
  const [taskPlannerDialogOpen, setTaskPlannerDialogOpen] = useState(false);
  const selectedModeRef = useRef<HTMLButtonElement>(null);
  const modeListRef = useRef<HTMLDivElement>(null);

  const handleModeSelect = useCallback(
    (mode: ChatModeInfo) => {
      setIsOpen(false);
      if (mode.id === "task_planner") {
        setTaskPlannerDialogOpen(true);
        return;
      }
      if (hasMessages) {
        setTargetModeForTransition(mode);
        setTransitionDialogOpen(true);
      } else {
        onModeChange(mode.id, mode.thread_defaults);
      }
    },
    [hasMessages, onModeChange],
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
    setIsOpen(false);
  };

  if (isLoading) {
    return (
      <Skeleton>
        <div className={styles.trigger}>
          <Text size="1">Loading...</Text>
        </div>
      </Skeleton>
    );
  }

  if (effectiveModes.length === 0) {
    return (
      <div className={`${styles.trigger} ${styles.disabled}`}>
        <Text size="1" color="gray">
          No modes
        </Text>
      </div>
    );
  }

  const triggerContent = (
    <Flex align="center" gap="1" className={styles.triggerContent}>
      <Text size="1">{currentTitle}</Text>
      {toolsCount > 0 && (
        <>
          <Text size="1" color="gray">
            ·
          </Text>
          <Text size="1" color="gray">
            {toolsCount} tools
          </Text>
        </>
      )}
    </Flex>
  );

  return (
    <>
      <Popover.Root open={isOpen} onOpenChange={setIsOpen}>
        <Popover.Trigger>
          <button
            className={`${styles.trigger} ${
              isModeDisabled ? styles.disabled : ""
            }`}
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
        >
          <div className={styles.modeList} ref={modeListRef}>
            {effectiveModes.map((mode, index) => {
              const isSelected = effectiveMode === mode.id;
              return (
                <React.Fragment key={mode.id}>
                  {index > 0 && (
                    <Separator size="4" className={styles.separator} />
                  )}
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
            <Separator size="4" className={styles.separator} />
            <button
              className={styles.addModeItem}
              onClick={handleCreateNewMode}
              type="button"
            >
              <Text size="1">Create new mode...</Text>
            </button>
          </div>
        </Popover.Content>
      </Popover.Root>

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
        className={`${styles.item} ${isSelected ? styles.itemSelected : ""} ${
          disabled ? styles.itemDisabled : ""
        }`}
        onClick={onSelect}
        type="button"
        disabled={disabled}
      >
        <Flex direction="column" gap="1" style={{ width: "100%" }}>
          <Flex align="center" gap="2">
            <Text size="1" weight="medium">
              {mode.title}
            </Text>
            {showTransitionHint && (
              <Badge
                size="1"
                color={isSelfSwitch ? "green" : "amber"}
                variant="soft"
              >
                {isSelfSwitch ? "restart" : "switch"}
              </Badge>
            )}
          </Flex>

          {mode.description && (
            <Text size="1" color="gray" className={styles.description}>
              {mode.description.length > 80
                ? mode.description.slice(0, 80) + "..."
                : mode.description}
            </Text>
          )}

          <Flex align="center" gap="1" wrap="wrap">
            {mode.ui.tags.slice(0, 2).map((tag) => (
              <Badge
                key={tag}
                size="1"
                color="gray"
                variant="soft"
                className={styles.badge}
              >
                {tag}
              </Badge>
            ))}
            {mode.tools_count > 0 && (
              <Badge
                size="1"
                color="blue"
                variant="soft"
                className={styles.badge}
              >
                {mode.tools_count} tools
              </Badge>
            )}
          </Flex>
        </Flex>
      </button>
    );
  },
);

ModeMenuItem.displayName = "ModeMenuItem";
ModeSelect.displayName = "ModeSelect";

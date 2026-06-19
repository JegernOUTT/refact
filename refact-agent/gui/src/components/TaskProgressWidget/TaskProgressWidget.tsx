import React, { useCallback, useEffect, useMemo, useState } from "react";
import * as Collapsible from "@radix-ui/react-collapsible";
import classNames from "classnames";
import { Pause, Play, Square, Target } from "lucide-react";

import { useChatActions, useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectCurrentTasksById,
  selectGoalById,
  selectHasTasksById,
  selectIsStreamingById,
  selectTaskGoalExpandedById,
  selectTaskProgressById,
  selectTasksEverUsedById,
  selectTaskWidgetExpandedById,
  selectThreadModeById,
  selectThreadToolUseById,
  setTaskGoalExpanded,
  setTaskWidgetExpanded,
  useThreadId,
} from "../../features/Chat/Thread";
import type { TodoItem, TodoStatus } from "../../features/Chat/Thread/types";
import type {
  GoalAttempt,
  GoalEvent,
  GoalSnapshot,
  GoalStatus,
} from "../../services/refact/types";
import { Box, Flex, Separator, Text } from "../LongTailPrimitives";
import { Badge, Button, Icon } from "../ui";
import { Chevron } from "../Collapsible";
import { CircularProgress } from "../CircularProgress/CircularProgress";
import { StatusDot, type StatusDotState } from "../StatusDot";
import {
  addBuddyCrashBreadcrumb,
  setBuddyCrashHotSlot,
} from "../../features/Buddy/reportBuddyFrontendError";
import styles from "./TaskProgressWidget.module.css";

function getStatusDotState(
  status: TodoStatus,
  _isStreaming: boolean,
): StatusDotState {
  switch (status) {
    case "in_progress":
      return "in_progress";
    case "completed":
      return "completed";
    case "failed":
      return "error";
    case "pending":
    default:
      return "idle";
  }
}

const STATUS_TOOLTIPS: Record<TodoStatus, string> = {
  completed: "Completed",
  in_progress: "In progress",
  pending: "Pending",
  failed: "Failed",
};

const GOAL_STATUS_LABELS: Record<GoalStatus, string> = {
  active: "Active",
  verifying: "Verifying",
  paused: "Paused",
  completed: "Completed",
  stopped: "Stopped",
  budget_exhausted: "Budget exhausted",
  no_progress: "No progress",
  transferred: "Transferred",
};

function goalStatusTone(
  status: GoalStatus,
): React.ComponentProps<typeof Badge>["tone"] {
  switch (status) {
    case "active":
    case "verifying":
      return "accent";
    case "completed":
      return "success";
    case "paused":
      return "warning";
    case "stopped":
    case "budget_exhausted":
    case "no_progress":
      return "danger";
    case "transferred":
    default:
      return "muted";
  }
}

function hasGoalWork(goal: GoalSnapshot | null): boolean {
  return goal !== null && goal.content.trim().length > 0;
}

const GOAL_SUPPORTED_MODES = new Set([
  "agent",
  "openai_agent",
  "quick_agent",
  "task_agent",
  "task_planner",
  "ultra_agent",
]);

function isGoalSupported(
  mode: string | undefined,
  toolUse: string | undefined,
): boolean {
  return (
    toolUse === "agent" ||
    (mode ? GOAL_SUPPORTED_MODES.has(mode.trim().toLowerCase()) : false)
  );
}

type StatusIconProps = {
  status: TodoStatus;
  isStreaming?: boolean;
};

const StatusIcon: React.FC<StatusIconProps> = ({
  status,
  isStreaming = false,
}) => {
  const dotState = getStatusDotState(status, isStreaming);
  return (
    <StatusDot
      state={dotState}
      size="small"
      tooltipText={STATUS_TOOLTIPS[status]}
    />
  );
};

type TaskRowProps = {
  task: TodoItem;
  isStreaming: boolean;
};

const TaskRow: React.FC<TaskRowProps> = ({ task, isStreaming }) => {
  const isActive = task.status === "in_progress";

  return (
    <Flex
      align="center"
      gap="2"
      className={classNames(styles.taskRow, { [styles.active]: isActive })}
    >
      <StatusIcon status={task.status} isStreaming={isStreaming && isActive} />
      <Text size="1" className={styles.taskText}>
        {task.content}
      </Text>
    </Flex>
  );
};

type GoalIndicatorProps = {
  goal: GoalSnapshot;
};

const GoalIndicator: React.FC<GoalIndicatorProps> = ({ goal }) => (
  <Flex align="center" gap="2" className={styles.goalIndicator}>
    <Icon icon={Target} size="sm" tone="accent" />
    <Text size="1" className={styles.goalIndicatorText}>
      Goal set
    </Text>
    <Badge tone={goalStatusTone(goal.status)} size="xs" variant="soft">
      {GOAL_STATUS_LABELS[goal.status]}
    </Badge>
  </Flex>
);

type GoalSectionProps = {
  goal: GoalSnapshot | null;
  expanded: boolean;
  onExpandedChange: (open: boolean) => void;
  onSave: (content: string, hasGoal: boolean) => void;
  onControl: (action: "pause" | "resume" | "stop") => void;
};

const GoalSection: React.FC<GoalSectionProps> = ({
  goal,
  expanded,
  onExpandedChange,
  onSave,
  onControl,
}) => {
  const [draft, setDraft] = useState(goal?.content ?? "");

  useEffect(() => {
    setDraft(goal?.content ?? "");
  }, [goal?.content]);

  const trimmedDraft = draft.trim();
  const hasGoal = goal !== null;
  const canSave =
    trimmedDraft.length > 0 && trimmedDraft !== (goal?.content ?? "");

  const handleSave = useCallback(() => {
    if (!canSave) return;
    onSave(trimmedDraft, hasGoal);
  }, [canSave, hasGoal, onSave, trimmedDraft]);

  return (
    <Collapsible.Root open={expanded} onOpenChange={onExpandedChange}>
      <Flex direction="column" gap="2" className={styles.goalBlock}>
        <Collapsible.Trigger asChild>
          <button className={styles.goalHeader} type="button">
            <Flex align="center" gap="2" className={styles.goalHeaderContent}>
              <Icon icon={Target} size="sm" tone="accent" />
              <Text size="1" weight="medium" className={styles.goalTitle}>
                Goal
              </Text>
              {goal ? (
                <Badge
                  tone={goalStatusTone(goal.status)}
                  size="xs"
                  variant="soft"
                >
                  {GOAL_STATUS_LABELS[goal.status]}
                </Badge>
              ) : (
                <Badge tone="muted" size="xs" variant="soft">
                  Not set
                </Badge>
              )}
            </Flex>
            <Chevron open={expanded} />
          </button>
        </Collapsible.Trigger>

        <Collapsible.Content>
          <Flex direction="column" gap="3" className={styles.goalBody}>
            <Flex direction="column" gap="2">
              <label className={styles.goalLabel} htmlFor="task-goal-input">
                Goal text
              </label>
              <textarea
                className={styles.goalInput}
                id="task-goal-input"
                value={draft}
                onChange={(event) => setDraft(event.currentTarget.value)}
                placeholder="Set a goal for this thread"
              />
              <Flex align="center" justify="between" gap="2" wrap="wrap">
                <Text size="1" color="gray" className={styles.goalProgress}>
                  {goal
                    ? `${goal.progress.turns_used}/${goal.budget.max_turns} turns · ${goal.progress.tokens_used}/${goal.budget.max_tokens} tokens`
                    : "Save to start tracking a goal"}
                </Text>
                <Button
                  size="sm"
                  variant="primary"
                  disabled={!canSave}
                  onClick={handleSave}
                >
                  Save
                </Button>
              </Flex>
            </Flex>

            {goal ? (
              <>
                <GoalControls goal={goal} onControl={onControl} />
                <GoalAttempts attempts={goal.attempts} />
                <GoalEvents events={goal.events} />
              </>
            ) : null}
          </Flex>
        </Collapsible.Content>
      </Flex>
    </Collapsible.Root>
  );
};

type GoalControlsProps = {
  goal: GoalSnapshot;
  onControl: (action: "pause" | "resume" | "stop") => void;
};

const GoalControls: React.FC<GoalControlsProps> = ({ goal, onControl }) => {
  const canPause = goal.active && goal.status !== "paused";
  const canResume = goal.status === "paused" || !goal.active;
  const canStop = goal.status !== "stopped" && goal.status !== "completed";

  return (
    <Flex align="center" gap="2" wrap="wrap" className={styles.goalControls}>
      <Button
        size="sm"
        variant="soft"
        leftIcon={Pause}
        disabled={!canPause}
        onClick={() => onControl("pause")}
      >
        Pause
      </Button>
      <Button
        size="sm"
        variant="soft"
        leftIcon={Play}
        disabled={!canResume}
        onClick={() => onControl("resume")}
      >
        Resume
      </Button>
      <Button
        size="sm"
        variant="danger"
        leftIcon={Square}
        disabled={!canStop}
        onClick={() => onControl("stop")}
      >
        Stop
      </Button>
    </Flex>
  );
};

type GoalAttemptsProps = {
  attempts: GoalAttempt[];
};

const GoalAttempts: React.FC<GoalAttemptsProps> = ({ attempts }) => {
  if (attempts.length === 0) {
    return (
      <Text size="1" color="gray">
        No verifier attempts yet
      </Text>
    );
  }

  return (
    <Flex direction="column" gap="2">
      <Text size="1" weight="medium">
        Verifier attempts
      </Text>
      <Flex direction="column" gap="2">
        {attempts.map((attempt) => (
          <div
            className={styles.attemptCard}
            key={`${attempt.at_ms}-${attempt.trigger}`}
          >
            <Flex align="center" gap="2" wrap="wrap">
              <Badge tone="accent" size="xs" variant="soft">
                {attempt.verdict}
              </Badge>
              <Text size="1" color="gray">
                {attempt.trigger}
              </Text>
            </Flex>
            {attempt.gaps.length > 0 ? (
              <ul className={styles.gapList}>
                {attempt.gaps.map((gap) => (
                  <li key={gap}>{gap}</li>
                ))}
              </ul>
            ) : null}
            <Text as="div" size="1" className={styles.verifierReply}>
              {attempt.verifier_reply}
            </Text>
          </div>
        ))}
      </Flex>
    </Flex>
  );
};

type GoalEventsProps = {
  events: GoalEvent[];
};

const GoalEvents: React.FC<GoalEventsProps> = ({ events }) => {
  if (events.length === 0) {
    return (
      <Text size="1" color="gray">
        No goal events yet
      </Text>
    );
  }

  return (
    <Flex direction="column" gap="2">
      <Text size="1" weight="medium">
        Goal events
      </Text>
      <div className={styles.eventList}>
        {events.map((event) => (
          <div
            className={styles.eventRow}
            key={`${event.at_ms}-${event.kind}-${event.text}`}
          >
            <Badge tone="muted" size="xs" variant="soft">
              {event.kind}
            </Badge>
            <Text size="1" className={styles.eventText}>
              {event.text}
            </Text>
          </div>
        ))}
      </div>
    </Flex>
  );
};

export const TaskProgressWidget: React.FC = () => {
  const dispatch = useAppDispatch();
  const chatId = useThreadId();
  const hasTasks = useAppSelector((state) => selectHasTasksById(state, chatId));
  const everUsed = useAppSelector((state) =>
    selectTasksEverUsedById(state, chatId),
  );
  const tasks = useAppSelector((state) =>
    selectCurrentTasksById(state, chatId),
  );
  const goal = useAppSelector((state) => selectGoalById(state, chatId));
  const isExpanded = useAppSelector((state) =>
    selectTaskWidgetExpandedById(state, chatId),
  );
  const goalExpanded = useAppSelector((state) =>
    selectTaskGoalExpandedById(state, chatId),
  );
  const threadMode = useAppSelector((state) =>
    selectThreadModeById(state, chatId),
  );
  const threadToolUse = useAppSelector((state) =>
    selectThreadToolUseById(state, chatId),
  );
  const isStreaming = useAppSelector((state) =>
    selectIsStreamingById(state, chatId),
  );
  const { done, total, activeTitle } = useAppSelector((state) =>
    selectTaskProgressById(state, chatId),
  );
  const { setGoal, updateGoal, controlGoal } = useChatActions(chatId);
  const hasGoal = hasGoalWork(goal);
  const goalSupported = isGoalSupported(threadMode, threadToolUse);
  const isFreshGoalOpportunity =
    !everUsed && !hasTasks && !hasGoal && goalSupported;
  const isTasksCleared = everUsed && !hasTasks && !hasGoal;
  const shouldRender = everUsed || hasGoal || goalSupported;

  const crashSummary = useMemo(() => {
    const taskSummary =
      total > 0 ? `${done}/${total} active=${activeTitle ?? "none"}` : null;
    const goalSummary = goal ? `goal=${goal.status}` : null;
    return [taskSummary, goalSummary].filter(Boolean).join(" ") || null;
  }, [activeTitle, done, goal, total]);

  useEffect(() => {
    setBuddyCrashHotSlot("tasks", crashSummary);
    if (crashSummary) {
      addBuddyCrashBreadcrumb("tasks", crashSummary);
    }
  }, [crashSummary]);

  const handleOpenChange = useCallback(
    (open: boolean) => {
      if (chatId) {
        dispatch(setTaskWidgetExpanded({ id: chatId, expanded: open }));
      }
    },
    [dispatch, chatId],
  );

  const handleGoalOpenChange = useCallback(
    (open: boolean) => {
      if (chatId) {
        dispatch(setTaskGoalExpanded({ id: chatId, expanded: open }));
      }
    },
    [dispatch, chatId],
  );

  const handleGoalSave = useCallback(
    (content: string, goalExists: boolean) => {
      void (goalExists ? updateGoal(content) : setGoal(content));
    },
    [setGoal, updateGoal],
  );

  const handleGoalControl = useCallback(
    (action: "pause" | "resume" | "stop") => {
      void controlGoal(action);
    },
    [controlGoal],
  );

  if (!shouldRender) return null;

  return (
    <Box className={styles.widget}>
      <Collapsible.Root open={isExpanded} onOpenChange={handleOpenChange}>
        <Collapsible.Trigger asChild>
          <button className={styles.header} type="button">
            <Flex align="center" gap="3" className={styles.headerInner}>
              <Flex align="center" gap="2" className={styles.headerMain}>
                {!isExpanded && hasGoal && goal ? (
                  <GoalIndicator goal={goal} />
                ) : null}

                {!isExpanded && hasTasks ? (
                  <>
                    <Flex gap="1" align="center">
                      {tasks.map((task) => (
                        <StatusIcon
                          key={task.id}
                          status={task.status}
                          isStreaming={
                            task.status === "in_progress" && isStreaming
                          }
                        />
                      ))}
                    </Flex>

                    <CircularProgress done={done} total={total} />

                    {activeTitle ? (
                      <Text size="1" color="gray" className={styles.activeHint}>
                        {activeTitle}
                      </Text>
                    ) : null}
                  </>
                ) : null}

                {!isExpanded && isFreshGoalOpportunity ? (
                  <Flex
                    align="center"
                    gap="2"
                    className={styles.goalAffordance}
                  >
                    <Icon icon={Target} size="sm" tone="accent" />
                    <Text
                      size="1"
                      weight="medium"
                      className={styles.goalAffordanceText}
                    >
                      Set a goal
                    </Text>
                  </Flex>
                ) : null}

                {!isExpanded && isTasksCleared ? (
                  <Text size="1" color="gray">
                    Tasks cleared
                  </Text>
                ) : null}

                {isExpanded ? (
                  <Text size="1" weight="medium">
                    Task Progress
                  </Text>
                ) : null}
              </Flex>

              <Chevron open={isExpanded} />
            </Flex>
          </button>
        </Collapsible.Trigger>

        <Collapsible.Content>
          <Flex direction="column" gap="3" className={styles.content}>
            <GoalSection
              goal={goal}
              expanded={goalExpanded}
              onExpandedChange={handleGoalOpenChange}
              onSave={handleGoalSave}
              onControl={handleGoalControl}
            />

            {hasTasks ? (
              <>
                <div className={styles.taskList}>
                  {tasks.map((task) => (
                    <div key={task.id} className={styles.taskRowEnter}>
                      <TaskRow task={task} isStreaming={isStreaming} />
                    </div>
                  ))}
                </div>
                <Separator size="4" />
                <Text size="1" color="gray">
                  {done}/{total} completed
                </Text>
              </>
            ) : (
              <Text size="1" color="gray">
                No active tasks
              </Text>
            )}
          </Flex>
        </Collapsible.Content>
      </Collapsible.Root>
    </Box>
  );
};

export default TaskProgressWidget;

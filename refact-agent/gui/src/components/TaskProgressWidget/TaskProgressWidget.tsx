import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Check,
  Circle,
  CircleDot,
  Info,
  Pause,
  Pencil,
  Play,
  Square,
  Target,
  X,
  type LucideIcon,
} from "lucide-react";

import { useChatCommandRequest } from "../../hooks/useChatCommandRequest";
import { selectConfig } from "../../features/Config/configSlice";
import { getEngineEndpointIdentity } from "../../services/refact/apiUrl";
import { selectGoalById } from "../../features/Chat/Thread/selectors";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useAppSelector } from "../../hooks/useAppSelector";
import {
  selectCurrentTasksById,
  selectGoalWithPlanHistoryById,
  selectHasTasksById,
  selectTaskProgressById,
  selectTasksEverUsedById,
  selectTaskWidgetExpandedById,
  selectThreadModeById,
  selectThreadToolUseById,
  setTaskWidgetExpanded,
  useThreadId,
} from "../../features/Chat/Thread";
import type { TodoItem, TodoStatus } from "../../features/Chat/Thread/types";
import type {
  GoalAttempt,
  GoalEvent,
  GoalSnapshot,
  GoalPlanSnapshot,
  GoalStatus,
} from "../../services/refact/types";
import type { GoalBudgetCommand } from "../../services/refact/chatCommands";
import { Icon } from "../ui";
import { Markdown } from "../Markdown";
import {
  KitBadge,
  KitButton,
  KitDialog,
  KitDialogCancel,
  KitDivider,
  KitIconButton,
  KitNote,
  KitSection,
  KitSliderRow,
  KitStaticRow,
  KitToggleRow,
  KitTooltip,
  type KitTone,
} from "../ComposerKit";
import {
  addBuddyCrashBreadcrumb,
  setBuddyCrashHotSlot,
} from "../../features/Buddy/reportBuddyFrontendError";
import { humanizeIdentifier } from "../../utils/displayNames";
import { TranscriptItem } from "../ChatContent/TranscriptItem/TranscriptItem";
import styles from "./TaskProgressWidget.module.css";

const TASK_ICONS: Record<TodoStatus, LucideIcon> = {
  completed: Check,
  in_progress: CircleDot,
  pending: Circle,
  failed: X,
};

const TASK_STATUS_LABELS: Record<TodoStatus, string> = {
  completed: "Completed",
  in_progress: "In progress",
  pending: "Pending",
  failed: "Failed",
};

const MAX_HEADER_DOTS = 8;

function goalStatusTone(status: GoalStatus): KitTone {
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

function isPositiveLimit(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value) && value > 0;
}

function formatCount(value: number): string {
  if (!Number.isFinite(value)) return "0";
  if (value >= 1_000_000) return `${+(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${+(value / 1_000).toFixed(1)}k`;
  return String(value);
}

type LimitKey = "max_turns" | "max_minutes" | "max_tokens";
type LimitDraft = Record<LimitKey, number | null>;

const LIMIT_KEYS: readonly LimitKey[] = [
  "max_turns",
  "max_minutes",
  "max_tokens",
];

const LIMIT_META: Record<
  LimitKey,
  {
    label: string;
    hint: string;
    min: number;
    max: number;
    step: number;
    fallback: number;
    unit: string;
  }
> = {
  max_turns: {
    label: "Max turns",
    hint: "Pause the goal after this many turns",
    min: 1,
    max: 100,
    step: 1,
    fallback: 20,
    unit: "turns",
  },
  max_minutes: {
    label: "Max minutes",
    hint: "Pause the goal after this much wall time",
    min: 5,
    max: 480,
    step: 5,
    fallback: 60,
    unit: "min",
  },
  max_tokens: {
    label: "Max tokens",
    hint: "Pause the goal after spending this many tokens",
    min: 10_000,
    max: 2_000_000,
    step: 10_000,
    fallback: 200_000,
    unit: "tokens",
  },
};

function limitDraftFromGoal(goal: GoalSnapshot | null): LimitDraft {
  const read = (key: LimitKey) => {
    const value = goal?.budget[key];
    return isPositiveLimit(value) ? value : null;
  };
  return {
    max_turns: read("max_turns"),
    max_minutes: read("max_minutes"),
    max_tokens: read("max_tokens"),
  };
}

function budgetCommandFromDraft(draft: LimitDraft): GoalBudgetCommand {
  const budget: GoalBudgetCommand = {};
  for (const key of LIMIT_KEYS) {
    const value = draft[key];
    if (value !== null && value > 0) budget[key] = value;
  }
  return budget;
}

function sameDraft(a: LimitDraft, b: LimitDraft): boolean {
  return LIMIT_KEYS.every((key) => a[key] === b[key]);
}

type GoalControlAction = "pause" | "resume" | "stop";

const GOAL_STOPPABLE_STATUSES: ReadonlySet<GoalStatus> = new Set<GoalStatus>([
  "active",
  "verifying",
  "paused",
  "budget_exhausted",
  "no_progress",
]);

const GOAL_RESUMABLE_STATUSES: ReadonlySet<GoalStatus> = new Set<GoalStatus>([
  "paused",
  "stopped",
]);

type GoalControlDescriptor = {
  key: GoalControlAction;
  label: string;
  icon: LucideIcon;
  disabled: boolean;
};

function goalControlActions(
  goal: GoalSnapshot,
  pending: boolean,
): GoalControlDescriptor[] {
  const canPause = goal.active && goal.status === "active";
  const canResume = GOAL_RESUMABLE_STATUSES.has(goal.status);
  const canStop = GOAL_STOPPABLE_STATUSES.has(goal.status);
  if (!canPause && !canResume && !canStop) return [];

  const primary: GoalControlDescriptor = canResume
    ? { key: "resume", label: "Resume", icon: Play, disabled: pending }
    : {
        key: "pause",
        label: "Pause",
        icon: Pause,
        disabled: pending || !canPause,
      };

  return [
    ...(canPause || canResume || goal.status === "verifying" ? [primary] : []),
    { key: "stop", label: "Stop", icon: Square, disabled: pending || !canStop },
  ];
}

const GOAL_SUPPORTED_MODES = new Set(["agent", "task_agent", "task_planner"]);

function isGoalSupported(
  mode: string | undefined,
  toolUse: string | undefined,
): boolean {
  return (
    toolUse === "agent" ||
    (mode ? GOAL_SUPPORTED_MODES.has(mode.trim().toLowerCase()) : false)
  );
}

function plainGoalLines(content: string): string[] {
  return content
    .split("\n")
    .map((part) => part.replace(/^[#>*\-\s]+/, "").trim())
    .filter((part) => part.length > 0);
}

function goalHeadlineOf(content: string): string {
  return plainGoalLines(content)[0] ?? "Goal";
}

function goalSummaryOf(content: string): string {
  return plainGoalLines(content).slice(1).join(" ");
}

const TaskRow: React.FC<{ task: TodoItem }> = ({ task }) => (
  <div className={styles.taskRow} data-status={task.status}>
    <Icon
      icon={TASK_ICONS[task.status]}
      size="sm"
      className={styles.taskIcon}
      aria-label={TASK_STATUS_LABELS[task.status]}
    />
    <span className={styles.taskText}>{task.content}</span>
  </div>
);

const GoalUsage: React.FC<{ goal: GoalSnapshot }> = ({ goal }) => {
  const { budget, progress } = goal;
  const parts = [
    {
      key: "turns",
      used: progress.turns_used,
      limit: budget.max_turns,
      text: (used: string, limit?: string) =>
        limit ? `${used}/${limit} turns` : `${used} turns`,
    },
    {
      key: "tokens",
      used: progress.tokens_used,
      limit: budget.max_tokens,
      text: (used: string, limit?: string) =>
        limit ? `${used}/${limit} tokens` : `${used} tokens`,
    },
  ];
  const limits = [
    budget.max_turns,
    budget.max_minutes,
    budget.max_tokens,
    budget.no_progress_turns,
  ].filter(isPositiveLimit).length;

  return (
    <>
      {parts.map((part, index) => {
        const limited = isPositiveLimit(part.limit);
        return (
          <React.Fragment key={part.key}>
            {index > 0 ? " · " : null}
            <span
              data-saturated={
                limited && part.used >= (part.limit ?? 0) ? "true" : undefined
              }
              className={styles.usagePart}
            >
              {part.text(
                formatCount(part.used),
                limited ? formatCount(part.limit ?? 0) : undefined,
              )}
            </span>
          </React.Fragment>
        );
      })}
      {" · "}
      {limits === 0
        ? "No limits"
        : `${limits} ${limits === 1 ? "limit" : "limits"}`}
    </>
  );
};

const LimitRows: React.FC<{
  draft: LimitDraft;
  disabled: boolean;
  onChange: (next: LimitDraft) => void;
}> = ({ draft, disabled, onChange }) => (
  <>
    {LIMIT_KEYS.map((key) => {
      const meta = LIMIT_META[key];
      const value = draft[key];
      return (
        <React.Fragment key={key}>
          <KitToggleRow
            label={meta.label}
            ariaLabel={meta.label}
            hint={meta.hint}
            meta={value === null ? "Unlimited" : undefined}
            checked={value !== null}
            disabled={disabled}
            onCheckedChange={(on) =>
              onChange({ ...draft, [key]: on ? meta.fallback : null })
            }
          />
          {value !== null ? (
            <KitSliderRow
              label="Limit"
              ariaLabel={`${meta.label} value`}
              value={value}
              min={Math.min(meta.min, value)}
              max={Math.max(meta.max, value)}
              step={meta.step}
              editable
              unit={meta.unit}
              onValueChange={(next) => onChange({ ...draft, [key]: next })}
            />
          ) : null}
        </React.Fragment>
      );
    })}
  </>
);

export function GoalPlanSection({ plan }: { plan: GoalPlanSnapshot }) {
  return (
    <section aria-label="Goal plan" className={styles.planSection}>
      <KitSection>Plan · v{plan.version}</KitSection>
      <div className={styles.prose}>
        <Markdown variant="tool">{plan.content}</Markdown>
      </div>
      {plan.checkpoint ? (
        <KitNote>
          Checkpoint {plan.checkpoint.revision} ·{" "}
          {plan.checkpoint.folded_deltas} folded updates
        </KitNote>
      ) : null}
      <KitSection>Plan history · {plan.delta_count} updates</KitSection>
      {plan.history?.length ? (
        plan.history.map((entry) => (
          <section
            key={entry.seq}
            aria-label={`Plan revision ${entry.seq}`}
            className={styles.prose}
          >
            <span className={styles.revision}>Revision {entry.seq}</span>
            <Markdown variant="tool">{entry.content}</Markdown>
          </section>
        ))
      ) : (
        <KitNote>Earlier updates are retained in conversation history.</KitNote>
      )}
    </section>
  );
}

const GoalAttempts: React.FC<{ attempts: GoalAttempt[] }> = ({ attempts }) =>
  attempts.length === 0 ? (
    <KitNote>No verifier attempts yet</KitNote>
  ) : (
    <>
      {attempts.map((attempt) => (
        <KitStaticRow
          key={`${attempt.at_ms}-${attempt.trigger}`}
          label={humanizeIdentifier(attempt.trigger)}
          badges={
            <KitBadge tone="accent">
              {humanizeIdentifier(attempt.verdict)}
            </KitBadge>
          }
          hint={
            <>
              {attempt.gaps.length > 0 ? (
                <ul className={styles.gapList}>
                  {attempt.gaps.map((gap) => (
                    <li key={gap}>{gap}</li>
                  ))}
                </ul>
              ) : null}
              <span className={styles.wrapText}>{attempt.verifier_reply}</span>
            </>
          }
        />
      ))}
    </>
  );

const GoalEvents: React.FC<{ events: GoalEvent[] }> = ({ events }) =>
  events.length === 0 ? (
    <KitNote>No goal events yet</KitNote>
  ) : (
    <>
      {events.map((event) => (
        <KitStaticRow
          key={`${event.at_ms}-${event.kind}-${event.text}`}
          label={humanizeIdentifier(event.kind)}
          hint={<span className={styles.wrapText}>{event.text}</span>}
        />
      ))}
    </>
  );

type GoalEditorDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  goal: GoalSnapshot | null;
  pending: boolean;
  error: string | null;
  onCreate: (content: string, budget?: GoalBudgetCommand) => Promise<boolean>;
  onUpdateText: (content: string) => Promise<boolean>;
};

const GoalEditorDialog: React.FC<GoalEditorDialogProps> = ({
  open,
  onOpenChange,
  goal,
  pending,
  error,
  onCreate,
  onUpdateText,
}) => {
  const [draft, setDraft] = useState(goal?.content ?? "");
  const [limits, setLimits] = useState<LimitDraft>(() =>
    limitDraftFromGoal(goal),
  );
  const goalContent = goal?.content;
  const wasOpen = useRef(false);
  useEffect(() => {
    if (open && !wasOpen.current) {
      setDraft(goalContent ?? "");
      setLimits(limitDraftFromGoal(null));
    }
    wasOpen.current = open;
  }, [open, goalContent]);

  const isUpdate = goal !== null;
  const trimmed = draft.trim();
  const canSave = trimmed.length > 0 && trimmed !== (goal?.content ?? "");

  const handleSave = async () => {
    if (!canSave || pending) return;
    const budget = budgetCommandFromDraft(limits);
    const ok = isUpdate
      ? await onUpdateText(trimmed)
      : await onCreate(
          trimmed,
          Object.keys(budget).length > 0 ? budget : undefined,
        );
    if (ok) onOpenChange(false);
  };

  return (
    <KitDialog
      open={open}
      onOpenChange={onOpenChange}
      title={isUpdate ? "Update goal" : "Set a goal"}
      description={
        isUpdate
          ? "Saved changes are appended as a goal update, not a replacement."
          : "The agent keeps working toward this goal across turns."
      }
      footerNote={error ?? undefined}
      footer={
        <>
          <KitDialogCancel />
          <KitButton
            variant="primary"
            disabled={!canSave || pending}
            onClick={() => void handleSave()}
          >
            {isUpdate ? "Update goal" : "Set goal"}
          </KitButton>
        </>
      }
    >
      <KitSection>Goal</KitSection>
      <textarea
        aria-label="Goal text"
        className={styles.goalInput}
        value={draft}
        disabled={pending}
        onChange={(event) => setDraft(event.currentTarget.value)}
        placeholder="Describe what done looks like"
      />
      {!isUpdate ? (
        <>
          <KitDivider />
          <KitSection>Limits</KitSection>
          <LimitRows draft={limits} disabled={pending} onChange={setLimits} />
        </>
      ) : null}
    </KitDialog>
  );
};

type GoalDetailsDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  goal: GoalSnapshot;
  pending: boolean;
  error: string | null;
  onApplyBudget: (budget: GoalBudgetCommand) => Promise<boolean>;
};

const GoalDetailsDialog: React.FC<GoalDetailsDialogProps> = ({
  open,
  onOpenChange,
  goal,
  pending,
  error,
  onApplyBudget,
}) => {
  const savedTurns = goal.budget.max_turns;
  const savedMinutes = goal.budget.max_minutes;
  const savedTokens = goal.budget.max_tokens;
  const saved = useMemo<LimitDraft>(
    () => ({
      max_turns: isPositiveLimit(savedTurns) ? savedTurns : null,
      max_minutes: isPositiveLimit(savedMinutes) ? savedMinutes : null,
      max_tokens: isPositiveLimit(savedTokens) ? savedTokens : null,
    }),
    [savedTurns, savedMinutes, savedTokens],
  );
  const [limits, setLimits] = useState<LimitDraft>(saved);
  useEffect(() => {
    if (open) setLimits(saved);
  }, [open, saved]);
  const dirty = !sameDraft(limits, saved);

  return (
    <KitDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Goal details"
      description="Plan, limits, verifier attempts and events for this goal."
      meta={
        <KitBadge tone={goalStatusTone(goal.status)}>
          {humanizeIdentifier(goal.status)}
        </KitBadge>
      }
      width="640px"
      maxHeight="80vh"
      footerNote={error ?? undefined}
      footer={
        <>
          <KitDialogCancel>Close</KitDialogCancel>
          <KitButton
            variant="primary"
            disabled={!dirty || pending}
            onClick={() => void onApplyBudget(budgetCommandFromDraft(limits))}
          >
            Apply limits
          </KitButton>
        </>
      }
    >
      <KitSection>Goal · v{goal.version}</KitSection>
      <div className={`${styles.prose} ${styles.goalProse}`}>
        <Markdown variant="tool">{goal.content}</Markdown>
      </div>
      {goal.status === "transferred" && goal.transferred_to ? (
        <KitNote>Transferred to {goal.transferred_to}</KitNote>
      ) : null}
      {goal.plan ? <GoalPlanSection plan={goal.plan} /> : null}
      <KitDivider />
      <KitSection>Limits</KitSection>
      <KitStaticRow
        label="Usage"
        meta={<GoalUsage goal={goal} />}
        tone="muted"
      />
      <LimitRows draft={limits} disabled={pending} onChange={setLimits} />
      <KitDivider />
      <KitSection>Verifier attempts · {goal.attempts.length}</KitSection>
      <GoalAttempts attempts={goal.attempts} />
      <KitSection>Events · {goal.events.length}</KitSection>
      <GoalEvents events={goal.events} />
    </KitDialog>
  );
};

type ActionButtonProps = {
  label: string;
  state?: string;
  icon: LucideIcon;
  disabled?: boolean;
  danger?: boolean;
  onClick: () => void;
};

const ActionButton: React.FC<ActionButtonProps> = ({
  label,
  state,
  icon,
  disabled,
  danger,
  onClick,
}) => (
  <KitTooltip label={label} state={state}>
    <KitIconButton
      aria-label={label}
      icon={icon}
      disabled={disabled}
      data-tone={danger ? "danger" : undefined}
      className={styles.action}
      onClick={onClick}
    />
  </KitTooltip>
);

export const TaskProgressWidget: React.FC = () => {
  const chatId = useThreadId();
  const config = useAppSelector(selectConfig);
  const principal = useRef({ key: config.apiKey, epoch: 0 });
  if (principal.current.key !== config.apiKey) {
    principal.current = {
      key: config.apiKey,
      epoch: principal.current.epoch + 1,
    };
  }
  const source = JSON.stringify([
    getEngineEndpointIdentity(config),
    chatId,
    principal.current.epoch,
  ]);
  return <TaskProgressWidgetInstance key={source} />;
};

const TASK_SECTION_LABELS = { output: "" };

const TaskProgressWidgetInstance: React.FC = () => {
  const dispatch = useAppDispatch();
  const chatId = useThreadId();
  const hasTasks = useAppSelector((state) => selectHasTasksById(state, chatId));
  const everUsed = useAppSelector((state) =>
    selectTasksEverUsedById(state, chatId),
  );
  const tasks = useAppSelector((state) =>
    selectCurrentTasksById(state, chatId),
  );
  const goal = useAppSelector((state) =>
    selectGoalWithPlanHistoryById(state, chatId),
  );
  const isExpanded = useAppSelector((state) =>
    selectTaskWidgetExpandedById(state, chatId),
  );
  const threadMode = useAppSelector((state) =>
    selectThreadModeById(state, chatId),
  );
  const threadToolUse = useAppSelector((state) =>
    selectThreadToolUseById(state, chatId),
  );
  const { done, total, activeTitle } = useAppSelector((state) =>
    selectTaskProgressById(state, chatId),
  );
  const request = useChatCommandRequest(chatId, "goal", (state) => {
    const current = selectGoalById(state, chatId);
    return (
      current?.version === goal?.version &&
      JSON.stringify(current?.owner) === JSON.stringify(goal?.owner)
    );
  });
  const [editorOpen, setEditorOpen] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);

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

  const sameGoal = (current: GoalSnapshot | null) =>
    current?.version === goal?.version &&
    JSON.stringify(current?.owner) === JSON.stringify(goal?.owner);

  const handleGoalCreate = (content: string, budget?: GoalBudgetCommand) =>
    request.run(
      { type: "set_goal", content, ...(budget ? { budget } : {}) },
      (state) => selectGoalById(state, chatId)?.content === content,
    );

  const handleGoalTextUpdate = (content: string) =>
    request.run({ type: "update_goal", note: content }, (state) => {
      const updated = selectGoalById(state, chatId);
      return (
        !!updated &&
        sameGoal(updated) &&
        updated.content !== goal?.content &&
        updated.content.endsWith(content)
      );
    });

  const handleGoalBudgetApply = (budget: GoalBudgetCommand) => {
    const nextBudget: GoalBudgetCommand = Object.fromEntries(
      Object.entries(goal?.budget ?? {}).filter(
        ([key, value]) =>
          typeof value === "number" &&
          key !== "max_turns" &&
          key !== "max_minutes" &&
          key !== "max_tokens",
      ),
    );
    Object.assign(nextBudget, budget);
    return request.run(
      { type: "set_goal_budget", budget: nextBudget },
      (state) => {
        const updated = selectGoalById(state, chatId);
        if (!updated || !sameGoal(updated)) return false;
        return LIMIT_KEYS.every(
          (key) => (updated.budget[key] ?? 0) === (nextBudget[key] ?? 0),
        );
      },
    );
  };

  const handleGoalControl = (action: GoalControlAction) => {
    const expected =
      action === "pause" ? "paused" : action === "stop" ? "stopped" : "active";
    void request.run({ type: "goal_control", action }, (state) => {
      const updated = selectGoalById(state, chatId);
      return sameGoal(updated) && updated?.status === expected;
    });
  };

  if (!shouldRender) return null;

  const activeGoal = hasGoal ? goal : null;
  const rowVerb = activeGoal
    ? "Goal"
    : isFreshGoalOpportunity
      ? "Set a goal"
      : isTasksCleared
        ? "Tasks cleared"
        : "Tasks";
  const rowTarget = activeGoal
    ? goalHeadlineOf(activeGoal.content)
    : activeTitle ?? undefined;
  const controls = activeGoal
    ? goalControlActions(activeGoal, request.blocked)
    : [];

  const headerMeta =
    activeGoal !== null || hasTasks ? (
      <span className={styles.headerMeta}>
        {activeGoal ? (
          <span
            className={styles.statusPill}
            data-tone={goalStatusTone(activeGoal.status)}
          >
            {humanizeIdentifier(activeGoal.status)}
          </span>
        ) : null}
        {hasTasks ? (
          <span
            className={styles.taskSummary}
            aria-label={`${done} of ${total} tasks completed`}
          >
            {tasks.length <= MAX_HEADER_DOTS ? (
              <span className={styles.taskDots} aria-hidden="true">
                {tasks.map((task) => (
                  <span
                    key={task.id}
                    className={styles.taskDot}
                    data-status={task.status}
                  />
                ))}
              </span>
            ) : null}
            <span>
              {done}/{total}
            </span>
          </span>
        ) : null}
      </span>
    ) : undefined;

  const goalSummary = activeGoal ? goalSummaryOf(activeGoal.content) : "";
  const body = (
    <div className={styles.body}>
      {activeGoal ? (
        goalSummary ? (
          <p className={styles.goalText} title={activeGoal.content}>
            {goalSummary}
          </p>
        ) : null
      ) : (
        <KitNote>
          {goalSupported
            ? "No goal set. Set one to keep the agent working toward it."
            : "No goal set."}
        </KitNote>
      )}
      {hasTasks ? (
        <div className={styles.taskList}>
          {tasks.map((task) => (
            <TaskRow key={task.id} task={task} />
          ))}
        </div>
      ) : (
        <KitNote>No active tasks</KitNote>
      )}
      <div className={styles.footer}>
        <span className={styles.footerMeta}>
          {activeGoal ? (
            <GoalUsage goal={activeGoal} />
          ) : hasTasks ? (
            `${done}/${total} completed`
          ) : null}
        </span>
        {activeGoal !== null || goalSupported ? (
          <ActionButton
            label={activeGoal ? "Edit goal" : "Create goal"}
            icon={activeGoal ? Pencil : Target}
            disabled={request.blocked}
            onClick={() => setEditorOpen(true)}
          />
        ) : null}
        {activeGoal ? (
          <ActionButton
            label="Goal details"
            state={
              activeGoal.plan ? `Plan v${activeGoal.plan.version}` : undefined
            }
            icon={Info}
            onClick={() => setDetailsOpen(true)}
          />
        ) : null}
        {controls.map((action) => (
          <ActionButton
            key={action.key}
            label={`${action.label} goal`}
            icon={action.icon}
            disabled={action.disabled}
            danger={action.key === "stop"}
            onClick={() => handleGoalControl(action.key)}
          />
        ))}
      </div>
    </div>
  );

  return (
    <div className={styles.widget}>
      <div className={styles.row}>
        <TranscriptItem
          icon={<Icon icon={Target} size="sm" tone="accent" />}
          verb={rowVerb}
          target={rowTarget}
          meta={headerMeta}
          expandable
          open={isExpanded}
          onOpenChange={handleOpenChange}
          sectionLabels={TASK_SECTION_LABELS}
          panelClassName={styles.panel}
          output={body}
        />
      </div>
      {goal?.status === "budget_exhausted" || goal?.status === "no_progress" ? (
        <p className={styles.feedback}>
          {goal.status === "budget_exhausted"
            ? "Budget exhausted. Review usage and explicitly adjust budget limits to continue."
            : "No progress detected. Provide new direction or review the progress budget before continuing."}
        </p>
      ) : null}
      {request.status ? (
        <p role="status" className={styles.feedback}>
          {request.status}
        </p>
      ) : null}
      {request.error ? (
        <p role="alert" className={styles.error}>
          {request.error}
        </p>
      ) : null}
      {request.uncertain ? (
        <div className={styles.feedback}>
          <KitButton
            disabled={request.pending}
            onClick={() => void request.checkStatus()}
          >
            Check status
          </KitButton>
        </div>
      ) : null}
      <GoalEditorDialog
        open={editorOpen}
        onOpenChange={setEditorOpen}
        goal={activeGoal}
        pending={request.blocked}
        error={request.error}
        onCreate={handleGoalCreate}
        onUpdateText={handleGoalTextUpdate}
      />
      {activeGoal ? (
        <GoalDetailsDialog
          open={detailsOpen}
          onOpenChange={setDetailsOpen}
          goal={activeGoal}
          pending={request.blocked}
          error={request.error}
          onApplyBudget={handleGoalBudgetApply}
        />
      ) : null}
    </div>
  );
};

export default TaskProgressWidget;

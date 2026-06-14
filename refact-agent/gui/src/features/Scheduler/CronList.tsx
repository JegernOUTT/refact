import React, { FormEvent, useState } from "react";
import { CalendarClock, Timer } from "lucide-react";
import {
  Badge,
  Button,
  EmptyState,
  Field,
  FieldError,
  FieldText,
  Icon,
  LoadingState,
  Surface,
} from "../../components/ui";
import type {
  CronTask,
  UpdateCronRequest,
} from "../../services/refact/schedulerApi";
import styles from "./Scheduler.module.css";

type CronListUpdate = Omit<UpdateCronRequest, "id">;

type CronListProps = {
  tasks: CronTask[];
  isLoading?: boolean;
  deletingId?: string | null;
  updatingId?: string | null;
  runningId?: string | null;
  onDelete: (id: string) => void;
  onToggleEnabled: (id: string, enabled: boolean) => void;
  onRunNow: (id: string) => void;
  onUpdate: (id: string, request: CronListUpdate) => void;
};

type EditableScheduleKind = "cron" | "interval" | "once";

type EditDraft = {
  kind: EditableScheduleKind;
  description: string;
  cron: string;
  every: string;
  at: string;
  tz: string;
};

type NextFireDisplay = {
  primary: string;
  absolute?: string;
  title?: string;
};

const CRON_PATTERN = /^\S+\s+\S+\s+\S+\s+\S+\s+\S+$/;
const INTERVAL_PATTERN = /^\d+\s*[smhd]$/;

function formatDuration(ms: number): string {
  const minutes = Math.max(1, Math.round(ms / 60000));
  const days = Math.floor(minutes / 1440);
  const hours = Math.floor((minutes % 1440) / 60);
  const mins = minutes % 60;

  if (days > 0) {
    return `in ${days}d${hours > 0 ? ` ${hours}h` : ""}`;
  }
  if (hours > 0) {
    return `in ${hours}h${mins > 0 ? ` ${mins}m` : ""}`;
  }
  return `in ${mins}m`;
}

function formatDurationInput(ms: number | null): string {
  if (!ms || ms <= 0) return "30m";
  const units = [
    { value: 24 * 60 * 60_000, suffix: "d" },
    { value: 60 * 60_000, suffix: "h" },
    { value: 60_000, suffix: "m" },
    { value: 1_000, suffix: "s" },
  ];
  const unit = units.find((item) => ms >= item.value && ms % item.value === 0);
  if (!unit) return `${Math.max(1, Math.round(ms / 60_000))}m`;
  return `${ms / unit.value}${unit.suffix}`;
}

function formatNextFire(timestampMs: number): NextFireDisplay {
  if (timestampMs <= 0) return { primary: "—" };

  const absolute = new Date(timestampMs).toLocaleString();
  const remaining = timestampMs - Date.now();
  if (remaining > 0) {
    return {
      primary: formatDuration(remaining),
      absolute,
      title: absolute,
    };
  }

  return { primary: absolute };
}

function lastRun(task: CronTask): CronTask["recent_runs"][number] | undefined {
  return task.recent_runs[task.recent_runs.length - 1];
}

function formatLastRun(task: CronTask): NextFireDisplay {
  const run = lastRun(task);
  if (!run) return { primary: "—" };
  const absolute = new Date(run.at_ms).toLocaleString();
  return { primary: absolute, title: absolute };
}

function statusTone(status: string | null) {
  const normalized = status?.toLowerCase() ?? "";
  if (["fired", "ok", "success"].includes(normalized)) return "success";
  if (normalized.includes("error") || normalized.includes("fail")) {
    return "danger";
  }
  if (normalized === "deferred" || normalized === "skipped") return "warning";
  return "muted";
}

function triggerLabel(task: CronTask): string {
  if (task.trigger_kind === "interval") return "Interval";
  if (task.trigger_kind === "once") return "One-shot";
  if (task.trigger_kind === "cron") return "Cron";
  return "Manual";
}

function actionLabel(task: CronTask): string {
  if (task.action_kind === "command") return "Command";
  if (task.isolated || task.target === "isolated") return "Isolated";
  return "Agent";
}

function actionTone(task: CronTask) {
  if (task.action_kind === "command") return "accent";
  if (task.isolated || task.target === "isolated") return "warning";
  return "default";
}

function scheduleCode(task: CronTask): string {
  if (task.cron) return task.cron;
  return task.human_schedule;
}

function editKind(task: CronTask): EditableScheduleKind {
  if (task.trigger_kind === "interval") return "interval";
  if (task.trigger_kind === "once") return "once";
  return "cron";
}

function createEditDraft(task: CronTask): EditDraft {
  return {
    kind: editKind(task),
    description: task.description,
    cron: task.cron || "7 * * * *",
    every: formatDurationInput(task.every_ms),
    at: task.at_ms ? new Date(task.at_ms).toISOString() : "in 30m",
    tz: task.tz ?? "",
  };
}

function validateDraft(draft: EditDraft): string | null {
  if (!draft.description.trim()) return "Description is required.";
  if (draft.description.length > 80) {
    return "Description must be 80 characters or less.";
  }
  if (draft.kind === "interval") {
    if (!draft.every.trim()) return "Interval is required.";
    if (!INTERVAL_PATTERN.test(draft.every.trim())) {
      return "Use a duration like 30m, 2h, or 1d.";
    }
  }
  if (draft.kind === "once" && !draft.at.trim()) {
    return "One-shot time is required.";
  }
  if (draft.kind === "cron") {
    if (!draft.cron.trim()) return "Cron expression is required.";
    if (!CRON_PATTERN.test(draft.cron.trim())) {
      return "Use a standard 5-field cron expression.";
    }
  }
  return null;
}

function draftUpdate(draft: EditDraft): CronListUpdate {
  const base = { description: draft.description.trim() };
  if (draft.kind === "interval") return { ...base, every: draft.every.trim() };
  if (draft.kind === "once") return { ...base, at: draft.at.trim() };
  return {
    ...base,
    cron: draft.cron.trim(),
    ...(draft.tz.trim() ? { tz: draft.tz.trim() } : {}),
  };
}

export const CronList: React.FC<CronListProps> = ({
  tasks,
  isLoading = false,
  deletingId = null,
  updatingId = null,
  runningId = null,
  onDelete,
  onToggleEnabled,
  onRunNow,
  onUpdate,
}) => {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState<EditDraft | null>(null);
  const [editError, setEditError] = useState<string | null>(null);

  if (isLoading) {
    return <LoadingState label="Loading scheduled prompts" />;
  }

  if (tasks.length === 0) {
    return (
      <EmptyState
        className={styles.emptyState}
        icon={CalendarClock}
        title="No scheduled prompts yet."
        description="Create a scheduled prompt to wake this chat on a schedule."
      />
    );
  }

  const beginEdit = (task: CronTask) => {
    setEditingId(task.id);
    setEditDraft(createEditDraft(task));
    setEditError(null);
  };

  const cancelEdit = () => {
    setEditingId(null);
    setEditDraft(null);
    setEditError(null);
  };

  const submitEdit = (event: FormEvent<HTMLFormElement>, id: string) => {
    event.preventDefault();
    if (!editDraft) return;
    const error = validateDraft(editDraft);
    if (error) {
      setEditError(error);
      return;
    }
    onUpdate(id, draftUpdate(editDraft));
    cancelEdit();
  };

  return (
    <div className={styles.jobList}>
      {tasks.map((task) => {
        const nextFire = formatNextFire(task.next_fire_at_ms);
        const lastFire = formatLastRun(task);
        const editing = editingId === task.id && editDraft;
        const updating = updatingId === task.id;
        const running = runningId === task.id;
        const deleting = deletingId === task.id;

        return (
          <Surface
            animated="rise"
            as="article"
            className={styles.jobCard}
            key={task.id}
            variant="surface-1"
          >
            <div className={styles.jobHeader}>
              <div className={styles.iconTile} aria-hidden="true">
                <Icon icon={Timer} tone="accent" />
              </div>
              <div className={styles.jobTitleBlock}>
                <h3 className={styles.jobTitle}>{task.human_schedule}</h3>
                <code className={styles.jobCron}>{scheduleCode(task)}</code>
              </div>
              <div className={styles.jobBadges}>
                <Badge tone={task.enabled ? "success" : "warning"}>
                  {task.enabled ? "Enabled" : "Paused"}
                </Badge>
                <Badge tone={statusTone(task.last_status)}>
                  {task.last_status ?? "Pending"}
                </Badge>
                <Badge tone="default">{triggerLabel(task)}</Badge>
                <Badge tone={actionTone(task)}>{actionLabel(task)}</Badge>
                <Badge tone={task.durable ? "accent" : "muted"}>
                  {task.durable ? "Durable" : "Session"}
                </Badge>
                <Badge tone={task.recurring ? "success" : "warning"}>
                  {task.recurring ? "Recurring" : "One-shot"}
                </Badge>
              </div>
            </div>

            <p className={styles.jobDescription}>{task.description}</p>

            <dl className={styles.jobMeta}>
              <div
                className={styles.jobMetaItem}
                title={nextFire.title ?? undefined}
              >
                <dt>Next fire</dt>
                <dd>
                  <span>{nextFire.primary}</span>
                  {nextFire.absolute ? (
                    <span className={styles.metaSecondary}>
                      {nextFire.absolute}
                    </span>
                  ) : null}
                </dd>
              </div>
              <div
                className={styles.jobMetaItem}
                title={lastFire.title ?? undefined}
              >
                <dt>Last fired</dt>
                <dd>{lastFire.primary}</dd>
              </div>
              <div className={styles.jobMetaItem}>
                <dt>Fires</dt>
                <dd>{task.fire_count}</dd>
              </div>
              {task.last_error ? (
                <div className={styles.jobMetaItem}>
                  <dt>Last error</dt>
                  <dd>{task.last_error}</dd>
                </div>
              ) : null}
            </dl>

            {editing ? (
              <form
                className={styles.editForm}
                onSubmit={(event) => submitEdit(event, task.id)}
              >
                <Field
                  label="Description"
                  helper={`${editDraft.description.length}/80`}
                  required
                >
                  <FieldText
                    className={styles.fieldControl}
                    maxLength={80}
                    value={editDraft.description}
                    onChange={(description) =>
                      setEditDraft({ ...editDraft, description })
                    }
                    aria-label="Edit description"
                  />
                </Field>
                {editDraft.kind === "cron" ? (
                  <div className={styles.editGrid}>
                    <Field
                      label="Cron expression"
                      helper="minute hour day month weekday"
                      required
                    >
                      <FieldText
                        className={styles.monoField}
                        value={editDraft.cron}
                        onChange={(cron) =>
                          setEditDraft({ ...editDraft, cron })
                        }
                        aria-label="Edit cron expression"
                      />
                    </Field>
                    <Field label="Timezone" helper="Optional IANA timezone.">
                      <FieldText
                        className={styles.fieldControl}
                        value={editDraft.tz}
                        onChange={(tz) => setEditDraft({ ...editDraft, tz })}
                        aria-label="Edit timezone"
                        placeholder="UTC"
                      />
                    </Field>
                  </div>
                ) : null}
                {editDraft.kind === "interval" ? (
                  <Field
                    label="Interval"
                    helper="Use s, m, h, or d suffixes."
                    required
                  >
                    <FieldText
                      className={styles.monoField}
                      value={editDraft.every}
                      onChange={(every) =>
                        setEditDraft({ ...editDraft, every })
                      }
                      aria-label="Edit interval"
                    />
                  </Field>
                ) : null}
                {editDraft.kind === "once" ? (
                  <Field
                    label="One-shot time"
                    helper="Use a relative time or RFC3339 timestamp."
                    required
                  >
                    <FieldText
                      className={styles.monoField}
                      value={editDraft.at}
                      onChange={(at) => setEditDraft({ ...editDraft, at })}
                      aria-label="Edit one-shot time"
                    />
                  </Field>
                ) : null}
                {editError ? <FieldError>{editError}</FieldError> : null}
                <div className={styles.jobActions}>
                  <Button
                    type="submit"
                    variant="primary"
                    size="sm"
                    loading={updating}
                  >
                    Save
                  </Button>
                  <Button
                    type="button"
                    variant="soft"
                    size="sm"
                    onClick={cancelEdit}
                  >
                    Cancel
                  </Button>
                </div>
              </form>
            ) : null}

            <div className={styles.jobActions}>
              <Button
                variant="soft"
                size="sm"
                loading={updating}
                disabled={deleting || running}
                onClick={() => onToggleEnabled(task.id, !task.enabled)}
              >
                {task.enabled ? "Pause" : "Resume"}
              </Button>
              <Button
                variant="soft"
                size="sm"
                loading={running}
                disabled={deleting || updating}
                onClick={() => onRunNow(task.id)}
              >
                Run now
              </Button>
              <Button
                variant="soft"
                size="sm"
                disabled={deleting || updating || running}
                onClick={() => beginEdit(task)}
              >
                Edit
              </Button>
              <Button
                variant="danger"
                size="sm"
                disabled={deleting}
                onClick={() => onDelete(task.id)}
              >
                Delete
              </Button>
            </div>
          </Surface>
        );
      })}
    </div>
  );
};

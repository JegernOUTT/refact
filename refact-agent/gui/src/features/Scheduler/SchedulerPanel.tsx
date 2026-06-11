import React, { useMemo, useState } from "react";
import { ArrowLeft, RefreshCw } from "lucide-react";
import {
  Badge,
  Button,
  FieldError,
  StatusDot,
  Surface,
} from "../../components/ui";
import { useAppSelector } from "../../hooks";
import {
  type CreateCronRequest,
  schedulerErrorMessage,
  useCreateCronMutation,
  useDeleteCronMutation,
  useGetCronTasksQuery,
} from "../../services/refact/schedulerApi";
import {
  selectCurrentThreadId,
  selectThreadMode,
} from "../Chat/Thread/selectors";
import { SettingsSection } from "../Settings/SettingsSection";
import { CronCreateForm } from "./CronCreateForm";
import { selectLastCronFireAt } from "./schedulerSlice";
import { CronList } from "./CronList";
import styles from "./Scheduler.module.css";

type SchedulerPanelProps = {
  onBack: () => void;
  embedded?: boolean;
};

export const SchedulerPanel: React.FC<SchedulerPanelProps> = ({
  onBack,
  embedded,
}) => {
  const {
    data: tasks = [],
    isFetching,
    error,
    refetch,
  } = useGetCronTasksQuery(undefined);
  const [createCron, createState] = useCreateCronMutation();
  const [deleteCron, deleteState] = useDeleteCronMutation();
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [deleteError, setDeleteError] = useState<unknown>(null);
  const lastCronFireAt = useAppSelector(selectLastCronFireAt);
  const currentThreadId = useAppSelector(selectCurrentThreadId);
  const currentMode = useAppSelector(selectThreadMode);

  const recurringCount = tasks.filter((task) => task.recurring).length;
  const durableCount = tasks.filter((task) => task.durable).length;

  const sortedTasks = useMemo(
    () =>
      [...tasks].sort((left, right) =>
        left.next_fire_at_ms === right.next_fire_at_ms
          ? left.id.localeCompare(right.id)
          : left.next_fire_at_ms - right.next_fire_at_ms,
      ),
    [tasks],
  );
  const renderedDeleteError = deleteState.error ?? deleteError;

  const handleCreate = async (
    request: Omit<CreateCronRequest, "chat_id" | "mode">,
  ) => {
    await createCron({
      ...request,
      chat_id: currentThreadId,
      mode: currentMode ?? undefined,
    }).unwrap();
  };

  const handleDelete = async (id: string) => {
    setDeletingId(id);
    setDeleteError(null);
    try {
      await deleteCron({ id }).unwrap();
    } catch (err) {
      setDeleteError(err);
    } finally {
      setDeletingId(null);
    }
  };

  const deleteTask = (id: string) => {
    handleDelete(id).catch(setDeleteError);
  };

  const actions = (
    <>
      {!embedded && (
        <Button variant="soft" onClick={onBack} leftIcon={ArrowLeft}>
          Back
        </Button>
      )}
      <Button
        variant="soft"
        onClick={() => void refetch()}
        leftIcon={RefreshCw}
      >
        Refresh
      </Button>
    </>
  );

  const summary = (
    <div className={styles.summaryBadges} aria-label="Scheduler summary">
      <Badge tone="default" variant="glass">
        <StatusDot status="idle" />
        {tasks.length} total
      </Badge>
      <Badge tone="success" variant="glass">
        <StatusDot status="success" />
        {recurringCount} recurring
      </Badge>
      <Badge tone="accent" variant="glass">
        <StatusDot status="running" />
        {durableCount} durable
      </Badge>
      {lastCronFireAt ? (
        <Badge tone="muted" variant="glass">
          Last fired {new Date(lastCronFireAt).toLocaleTimeString()}
        </Badge>
      ) : null}
    </div>
  );

  return (
    <SettingsSection
      title="Scheduler"
      description="Create, review, and delete scheduled prompts for the current chat."
      actions={actions}
      subNav={summary}
    >
      <div className={styles.panel}>
        <div className={styles.layout}>
          <Surface
            className={styles.createCard}
            variant="glass"
            animated="rise"
          >
            <h3 className={styles.paneTitle}>New schedule</h3>
            <CronCreateForm
              onSubmit={handleCreate}
              isLoading={createState.isLoading}
              error={createState.error}
              taskCount={tasks.length}
            />
          </Surface>

          <section
            className={styles.listPane}
            aria-labelledby="scheduler-list-title"
          >
            <div className={styles.listHeader}>
              <div className={styles.listTitleBlock}>
                <h3 className={styles.paneTitle} id="scheduler-list-title">
                  Scheduled prompts
                </h3>
                <p className={styles.sectionHint}>
                  Review the next fire time, schedule scope, recurrence, and
                  prompt description.
                </p>
              </div>
            </div>
            {error ? (
              <FieldError>{schedulerErrorMessage(error)}</FieldError>
            ) : null}
            {renderedDeleteError ? (
              <FieldError>
                {schedulerErrorMessage(renderedDeleteError)}
              </FieldError>
            ) : null}
            <CronList
              tasks={sortedTasks}
              isLoading={isFetching}
              deletingId={deletingId}
              onDelete={deleteTask}
            />
          </section>
        </div>
      </div>
    </SettingsSection>
  );
};

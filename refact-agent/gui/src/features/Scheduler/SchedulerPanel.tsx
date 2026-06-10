import React, { useMemo, useState } from "react";
import { ArrowLeft, RefreshCw } from "lucide-react";
import { Badge, Button, FieldError } from "../../components/ui";
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
import { SettingsGroup, SettingsSection } from "../Settings/SettingsSection";
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
      <Badge tone="default">{tasks.length} total</Badge>
      <Badge tone="success">{recurringCount} recurring</Badge>
      <Badge tone="accent">{durableCount} durable</Badge>
    </div>
  );

  return (
    <SettingsSection
      title="Scheduler"
      description="Create, review, and delete scheduled prompts for the current chat."
      actions={actions}
      subNav={summary}
    >
      <SettingsGroup title="Create schedule">
        <CronCreateForm
          onSubmit={handleCreate}
          isLoading={createState.isLoading}
          error={createState.error}
          taskCount={tasks.length}
        />
      </SettingsGroup>
      <SettingsGroup title="Scheduled prompts">
        <div className={styles.sectionStack}>
          <div className={styles.listHeader}>
            <p className={styles.sectionHint}>
              Review the next fire time, schedule scope, recurrence, and prompt
              description.
            </p>
            {lastCronFireAt ? (
              <span className={styles.lastFired}>
                Last fired {new Date(lastCronFireAt).toLocaleTimeString()}
              </span>
            ) : null}
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
        </div>
      </SettingsGroup>
    </SettingsSection>
  );
};

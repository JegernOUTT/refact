import React, { useMemo, useState } from "react";
import { ArrowLeft, RefreshCw } from "lucide-react";
import { Button, FieldError } from "../../components/ui";
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

export const SchedulerPanel: React.FC<SchedulerPanelProps> = ({ onBack, embedded }) => {
  const {
    data: tasks = [],
    isFetching,
    error,
    refetch,
  } = useGetCronTasksQuery(undefined);
  const [createCron, createState] = useCreateCronMutation();
  const [deleteCron, deleteState] = useDeleteCronMutation();
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const lastCronFireAt = useAppSelector(selectLastCronFireAt);
  const currentThreadId = useAppSelector(selectCurrentThreadId);
  const currentMode = useAppSelector(selectThreadMode);

  const sortedTasks = useMemo(
    () =>
      [...tasks].sort((left, right) =>
        left.next_fire_at_ms === right.next_fire_at_ms
          ? left.id.localeCompare(right.id)
          : left.next_fire_at_ms - right.next_fire_at_ms,
      ),
    [tasks],
  );

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
    try {
      await deleteCron({ id }).unwrap();
    } finally {
      setDeletingId(null);
    }
  };

  const deleteTask = (id: string) => {
    void handleDelete(id);
  };

  const actions = (
    <>
      {!embedded && (
        <Button variant="soft" onClick={onBack} leftIcon={ArrowLeft}>
          Back
        </Button>
      )}
      <Button variant="soft" onClick={() => void refetch()} leftIcon={RefreshCw}>
        Refresh
      </Button>
    </>
  );

  return (
    <SettingsSection
      title="Scheduler"
      description="Create, review, and delete cron prompts."
      actions={actions}
    >
      <CronCreateForm
        onSubmit={handleCreate}
        isLoading={createState.isLoading}
        error={createState.error}
        taskCount={tasks.length}
      />
      <SettingsGroup title="Scheduled prompts">
        <div className={styles.sectionStack}>
          <div className={styles.listHeader}>
            <p className={styles.sectionHint}>
              Human schedule, next fire, scope, and delete actions.
            </p>
            {lastCronFireAt ? (
              <span className={styles.lastFired}>
                Last fired {new Date(lastCronFireAt).toLocaleTimeString()}
              </span>
            ) : null}
          </div>
          {error ? <FieldError>{schedulerErrorMessage(error)}</FieldError> : null}
          {deleteState.error ? (
            <FieldError>{schedulerErrorMessage(deleteState.error)}</FieldError>
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

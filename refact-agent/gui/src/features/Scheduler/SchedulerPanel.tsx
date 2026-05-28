import React, { useMemo, useState } from "react";
import { Button, Card, Flex, Heading, Text } from "@radix-ui/themes";
import { ArrowLeftIcon, ReloadIcon } from "@radix-ui/react-icons";
import { useAppSelector } from "../../hooks";
import {
  type CreateCronRequest,
  schedulerErrorMessage,
  useCreateCronMutation,
  useDeleteCronMutation,
  useGetCronTasksQuery,
} from "../../services/refact/schedulerApi";
import { selectLastCronFireAt } from "./schedulerSlice";
import { CronCreateForm } from "./CronCreateForm";
import { CronList } from "./CronList";
import styles from "./Scheduler.module.css";

type SchedulerPanelProps = {
  onBack: () => void;
};

export const SchedulerPanel: React.FC<SchedulerPanelProps> = ({ onBack }) => {
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

  const sortedTasks = useMemo(
    () =>
      [...tasks].sort((left, right) =>
        left.next_fire_at_ms === right.next_fire_at_ms
          ? left.id.localeCompare(right.id)
          : left.next_fire_at_ms - right.next_fire_at_ms,
      ),
    [tasks],
  );

  const handleCreate = async (request: CreateCronRequest) => {
    await createCron(request).unwrap();
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

  return (
    <div className={styles.panel}>
      <div className={styles.header}>
        <Button variant="outline" onClick={onBack}>
          <ArrowLeftIcon width="16" height="16" />
          Back
        </Button>
        <Heading size="5">⏰ Scheduler</Heading>
        <Button variant="soft" onClick={() => void refetch()}>
          <ReloadIcon width="16" height="16" />
          Refresh
        </Button>
      </div>
      <div className={styles.content}>
        <CronCreateForm
          onSubmit={handleCreate}
          isLoading={createState.isLoading}
          error={createState.error}
          taskCount={tasks.length}
        />
        <Card>
          <Flex direction="column" gap="3">
            <Flex justify="between" align="center">
              <Text size="4" weight="bold">
                Scheduled prompts
              </Text>
              {lastCronFireAt && (
                <Text size="1" color="gray">
                  Last fired {new Date(lastCronFireAt).toLocaleTimeString()}
                </Text>
              )}
            </Flex>
            {error && (
              <Text className={styles.error} role="alert" size="2">
                {schedulerErrorMessage(error)}
              </Text>
            )}
            {deleteState.error && (
              <Text className={styles.error} role="alert" size="2">
                {schedulerErrorMessage(deleteState.error)}
              </Text>
            )}
            <CronList
              tasks={sortedTasks}
              isLoading={isFetching}
              deletingId={deletingId}
              onDelete={deleteTask}
            />
          </Flex>
        </Card>
      </div>
    </div>
  );
};

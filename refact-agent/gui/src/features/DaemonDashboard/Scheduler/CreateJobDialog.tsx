import { useEffect, useState } from "react";

import {
  Button,
  Dialog,
  FieldStack,
  FieldSelect,
  type FieldSelectOption,
} from "../../../components/ui";
import { CronCreateForm } from "../../Scheduler/CronCreateForm";
import type { CreateCronRequest } from "../../../services/refact/schedulerApi";
import type { DaemonWorker } from "../../../services/refact/daemon";
import { isReadyWorker } from "../Projects/projectRagStatus";
import { createProjectCron } from "./schedulerFanout";
import styles from "./Scheduler.module.css";

type CreateJobRequest = Omit<CreateCronRequest, "chat_id" | "mode">;

type CreateJobDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  daemonBase: string;
  workers: DaemonWorker[];
  taskCounts: Record<string, number | undefined>;
  wakingProjectId: string | null;
  onCreated: (projectId: string) => void;
  onWake: (projectId: string) => void;
};

function workerOptions(workers: DaemonWorker[]): FieldSelectOption[] {
  return workers.map((worker) => ({
    value: worker.project_id,
    label: isReadyWorker(worker) ? worker.slug : `${worker.slug} (stopped)`,
  }));
}

export function CreateJobDialog({
  open,
  onOpenChange,
  daemonBase,
  workers,
  taskCounts,
  wakingProjectId,
  onCreated,
  onWake,
}: CreateJobDialogProps) {
  const [projectId, setProjectId] = useState("");
  const [isCreating, setIsCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  const selected = workers.find((worker) => worker.project_id === projectId);

  useEffect(() => {
    if (selected !== undefined || workers.length === 0) return;
    const firstReady = workers.find(isReadyWorker) ?? workers[0];
    setProjectId(firstReady.project_id);
  }, [selected, workers]);

  function handleOpenChange(nextOpen: boolean) {
    if (!nextOpen) setCreateError(null);
    onOpenChange(nextOpen);
  }

  async function handleCreate(request: CreateJobRequest) {
    if (!selected) return;
    setIsCreating(true);
    setCreateError(null);
    try {
      await createProjectCron(daemonBase, selected.project_id, {
        ...request,
        chat_id: "",
      });
      onCreated(selected.project_id);
      onOpenChange(false);
    } catch (error) {
      setCreateError(error instanceof Error ? error.message : "Create failed");
    } finally {
      setIsCreating(false);
    }
  }

  const selectedReady = selected !== undefined && isReadyWorker(selected);

  return (
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <Dialog.Content maxWidth="calc(var(--rf-space-6) * 22)">
        <Dialog.Title>New scheduled job</Dialog.Title>
        <Dialog.Description>
          Pick a project, then build the job. Agent jobs created here need the
          isolated session option because no chat is attached.
        </Dialog.Description>

        <div className={styles.dialogBody}>
          <FieldStack
            label="Project"
            control={
              <FieldSelect
                aria-label="Project"
                onChange={setProjectId}
                options={workerOptions(workers)}
                placeholder="Select a project"
                value={projectId}
              />
            }
          />

          {selected && !selectedReady ? (
            <div className={styles.wakeHint}>
              <p className={styles.muted}>
                This project&apos;s worker is stopped. Wake it to create a job.
              </p>
              <Button
                loading={wakingProjectId === selected.project_id}
                onClick={() => onWake(selected.project_id)}
                size="sm"
                variant="soft"
              >
                Wake project
              </Button>
            </div>
          ) : null}

          {selectedReady ? (
            <CronCreateForm
              error={createError ? { error: createError } : undefined}
              isLoading={isCreating}
              onSubmit={handleCreate}
              taskCount={taskCounts[selected.project_id] ?? 0}
            />
          ) : null}
        </div>
      </Dialog.Content>
    </Dialog>
  );
}

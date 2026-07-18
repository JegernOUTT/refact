import { useState } from "react";
import { Pin, PinOff, RefreshCw, Square, Trash2 } from "lucide-react";

import {
  Button,
  Dialog,
  FieldError,
  Surface,
} from "../../../../../components/ui";
import { useAppDispatch } from "../../../../../hooks";
import {
  useForgetProjectMutation,
  usePinProjectMutation,
  useRestartProjectMutation,
  useStopProjectMutation,
  type DaemonWorker,
} from "../../../../../services/refact/daemon";
import { navigateDashboard } from "../../../dashboardSlice";
import { workerStateName } from "../../projectRagStatus";
import { Fact } from "./shared";
import styles from "../ProjectDetail.module.css";

type SettingsTabProps = {
  worker: DaemonWorker;
  logsUrl: string;
  onMutated: () => void;
};

function mutationError(error: unknown): string | null {
  if (!error || typeof error !== "object") return null;
  if ("data" in error) {
    const data = error.data;
    if (typeof data === "string") return data;
    if (data && typeof data === "object") {
      if ("error" in data && typeof data.error === "string") return data.error;
      if ("detail" in data && typeof data.detail === "string")
        return data.detail;
    }
  }
  if ("message" in error && typeof error.message === "string") {
    return error.message;
  }
  return "Project action failed.";
}

export function SettingsTab({ worker, logsUrl, onMutated }: SettingsTabProps) {
  const dispatch = useAppDispatch();
  const [forgetOpen, setForgetOpen] = useState(false);
  const [pin, pinState] = usePinProjectMutation();
  const [restart, restartState] = useRestartProjectMutation();
  const [stop, stopState] = useStopProjectMutation();
  const [forget, forgetState] = useForgetProjectMutation();
  const workerState = workerStateName(worker);
  const pending =
    pinState.isLoading ||
    restartState.isLoading ||
    stopState.isLoading ||
    forgetState.isLoading;
  const error =
    mutationError(pinState.error) ??
    mutationError(restartState.error) ??
    mutationError(stopState.error);

  async function mutate(action: () => Promise<unknown>) {
    try {
      await action();
      onMutated();
    } catch {
      return;
    }
  }

  return (
    <div className={styles.tabBody}>
      <Surface className={styles.section} radius="card" variant="glass">
        <h3 className={styles.sectionTitle}>Paths</h3>
        <dl className={styles.factGrid}>
          <Fact label="Project root" value={worker.root} mono />
          <Fact
            label="Worker log"
            value={
              <a className={styles.mono} href={logsUrl}>
                daemon log tail
              </a>
            }
          />
        </dl>
      </Surface>
      <Surface className={styles.section} radius="card" variant="glass">
        <h3 className={styles.sectionTitle}>Worker controls</h3>
        <div className={styles.actions}>
          <Button
            disabled={pending}
            leftIcon={worker.pinned ? PinOff : Pin}
            loading={pinState.isLoading}
            onClick={() =>
              void mutate(() =>
                pin({
                  projectId: worker.project_id,
                  pinned: !worker.pinned,
                }).unwrap(),
              )
            }
            size="sm"
            variant="soft"
          >
            {worker.pinned ? "Unpin" : "Pin"}
          </Button>
          <Button
            disabled={pending}
            leftIcon={RefreshCw}
            loading={restartState.isLoading}
            onClick={() =>
              void mutate(() => restart(worker.project_id).unwrap())
            }
            size="sm"
            variant="soft"
          >
            Restart
          </Button>
          <Button
            disabled={pending || workerState === "stopped"}
            leftIcon={Square}
            loading={stopState.isLoading}
            onClick={() => void mutate(() => stop(worker.project_id).unwrap())}
            size="sm"
            variant="soft"
          >
            Stop
          </Button>
          <Button
            disabled={pending}
            leftIcon={Trash2}
            onClick={() => setForgetOpen(true)}
            size="sm"
            variant="danger"
          >
            Forget
          </Button>
        </div>
        {error ? <FieldError>{error}</FieldError> : null}
      </Surface>

      <Dialog open={forgetOpen} onOpenChange={setForgetOpen}>
        <Dialog.Content maxWidth="calc(var(--rf-space-6) * 12)">
          <Dialog.Title>Forget {worker.slug}?</Dialog.Title>
          <Dialog.Description>
            This stops its worker and removes the project from the dashboard.
          </Dialog.Description>
          {forgetState.error ? (
            <FieldError>{mutationError(forgetState.error)}</FieldError>
          ) : null}
          <div className={styles.dialogActions}>
            <Dialog.Close asChild>
              <Button disabled={forgetState.isLoading} variant="soft">
                Cancel
              </Button>
            </Dialog.Close>
            <Button
              disabled={forgetState.isLoading}
              loading={forgetState.isLoading}
              onClick={() =>
                void mutate(async () => {
                  await forget(worker.project_id).unwrap();
                  setForgetOpen(false);
                  dispatch(navigateDashboard({ page: "projects", params: {} }));
                })
              }
              variant="danger"
            >
              Forget project
            </Button>
          </div>
        </Dialog.Content>
      </Dialog>
    </div>
  );
}

import type { ReactNode } from "react";
import { Power } from "lucide-react";

import { Button, EmptyState, FieldError } from "../../../../components/ui";
import {
  useRestartProjectMutation,
  type DaemonWorker,
} from "../../../../services/refact/daemon";
import { mutationError } from "./mutationError";
import { isReadyWorker } from "../projectRagStatus";
import styles from "./ProjectDetail.module.css";

type WorkerGateProps = {
  worker: DaemonWorker;
  onMutated: () => void;
  children: ReactNode;
};

export function WorkerGate({ worker, onMutated, children }: WorkerGateProps) {
  const [restart, restartState] = useRestartProjectMutation();

  if (isReadyWorker(worker)) {
    return <>{children}</>;
  }

  const error = restartState.error ? mutationError(restartState.error) : null;

  return (
    <EmptyState
      action={
        <div className={styles.gateAction}>
          <Button
            loading={restartState.isLoading}
            onClick={() =>
              void restart(worker.project_id)
                .unwrap()
                .then(onMutated)
                .catch(() => undefined)
            }
            variant="primary"
          >
            Start worker
          </Button>
          {error ? <FieldError>{error}</FieldError> : null}
        </div>
      }
      description="Start the worker to load this project data."
      icon={Power}
      title="Worker is stopped"
    />
  );
}

import type { ReactNode } from "react";
import { Power } from "lucide-react";

import { Button, EmptyState } from "../../../../components/ui";
import {
  useRestartProjectMutation,
  type DaemonWorker,
} from "../../../../services/refact/daemon";
import { isReadyWorker } from "../projectRagStatus";

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

  return (
    <EmptyState
      action={
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
      }
      description="Start the worker to load this project data."
      icon={Power}
      title="Worker is stopped"
    />
  );
}

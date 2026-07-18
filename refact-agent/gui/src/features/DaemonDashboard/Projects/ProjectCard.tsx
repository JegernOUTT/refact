import { useState } from "react";
import {
  ExternalLink,
  Pin,
  PinOff,
  RefreshCw,
  Square,
  Trash2,
} from "lucide-react";

import {
  Badge,
  Button,
  Card,
  Dialog,
  FieldError,
} from "../../../components/ui";
import {
  useForgetProjectMutation,
  usePinProjectMutation,
  useRestartProjectMutation,
  useStopProjectMutation,
  type DaemonWorker,
} from "../../../services/refact/daemon";
import type { ProjectRagStatus } from "./projectRagStatus";
import { workerStateName } from "./projectRagStatus";
import styles from "./Projects.module.css";

type ProjectCardProps = {
  daemonBase: string;
  worker: DaemonWorker;
  ragStatus?: ProjectRagStatus;
  onMutated: () => void;
};

type WorkerPresentation = {
  label: string;
  tone: "success" | "warning" | "danger" | "muted";
};

function workerPresentation(worker: DaemonWorker): WorkerPresentation {
  switch (workerStateName(worker)) {
    case "ready":
      return { label: "Ready", tone: "success" };
    case "starting":
      return { label: "Starting", tone: "warning" };
    case "stopping":
      return { label: "Stopping", tone: "warning" };
    case "crashed":
    case "failed":
      return { label: "Crashed", tone: "danger" };
    default:
      return { label: "Stopped", tone: "muted" };
  }
}

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

function CodeGraphStatus({ status }: { status: ProjectRagStatus }) {
  if (status.state === "loading") return <>CodeGraph: loading…</>;
  if (status.state === "error") return <>worker asleep — open to wake</>;
  const codegraph = status.data.codegraph;
  if (!codegraph) return <>CodeGraph: unavailable</>;
  switch (codegraph.state) {
    case "indexing":
      return <>CodeGraph: indexing · queued {codegraph.queued}</>;
    case "working":
      return <>CodeGraph: working ✓</>;
    case "error":
      return <>CodeGraph: error</>;
    case "turned_off":
      return <>CodeGraph: off</>;
  }
}

function VecDbStatus({ status }: { status: ProjectRagStatus }) {
  if (status.state !== "ready") return null;
  const vecdb = status.data.vecdb;
  if (!vecdb) {
    return <span>VecDB: unavailable</span>;
  }
  if (status.data.vec_db_error) {
    return <span>VecDB: error</span>;
  }
  if (vecdb.state === "done" || vecdb.state === "cooldown") {
    return <span>VecDB: ready ✓</span>;
  }
  return (
    <span>
      VecDB: {vecdb.state} · {vecdb.files_unprocessed} queued
    </span>
  );
}

export function ProjectCard({
  daemonBase,
  worker,
  ragStatus,
  onMutated,
}: ProjectCardProps) {
  const [forgetOpen, setForgetOpen] = useState(false);
  const [restart, restartState] = useRestartProjectMutation();
  const [stop, stopState] = useStopProjectMutation();
  const [pin, pinState] = usePinProjectMutation();
  const [forget, forgetState] = useForgetProjectMutation();
  const presentation = workerPresentation(worker);
  const workerState = workerStateName(worker);
  const optimistic = worker.project_id.startsWith("pending:");
  const pending =
    optimistic ||
    restartState.isLoading ||
    stopState.isLoading ||
    pinState.isLoading ||
    forgetState.isLoading;
  const error =
    mutationError(restartState.error) ??
    mutationError(stopState.error) ??
    mutationError(pinState.error) ??
    mutationError(forgetState.error);
  const openUrl = `${daemonBase.replace(/\/+$/, "")}/p/${encodeURIComponent(
    worker.project_id,
  )}/`;

  async function mutate(action: () => Promise<unknown>) {
    try {
      await action();
      onMutated();
    } catch {
      return;
    }
  }

  return (
    <Card
      aria-label={`${worker.slug} project`}
      className={styles.card}
      padding="lg"
      variant="surface-1"
    >
      <div className={styles.cardHeader}>
        <div className={styles.cardIdentity}>
          <h3 className={styles.cardTitle}>{worker.slug}</h3>
          <span className={styles.path} title={worker.root}>
            {worker.root}
          </span>
        </div>
        <Button
          aria-label={
            worker.pinned ? `Unpin ${worker.slug}` : `Pin ${worker.slug}`
          }
          disabled={pending}
          leftIcon={worker.pinned ? PinOff : Pin}
          onClick={() =>
            void mutate(() =>
              pin({
                projectId: worker.project_id,
                pinned: !worker.pinned,
              }).unwrap(),
            )
          }
          size="sm"
          variant="ghost"
        >
          {worker.pinned ? "Unpin" : "Pin"}
        </Button>
      </div>

      <div className={styles.statusRow}>
        <Badge tone={presentation.tone} variant="soft">
          {presentation.label}
        </Badge>
        {workerState === "stopped" ? (
          <span className={styles.muted}>Starts when you open it</span>
        ) : null}
      </div>

      <div
        className={styles.indexStatus}
        aria-label={`${worker.slug} index status`}
      >
        {workerState === "ready" ? (
          ragStatus ? (
            <>
              <span>
                <CodeGraphStatus status={ragStatus} />
              </span>
              <VecDbStatus status={ragStatus} />
            </>
          ) : (
            <span>CodeGraph: loading…</span>
          )
        ) : (
          <span>Indexes available when the worker is ready</span>
        )}
      </div>

      <div className={styles.stats}>
        <span>LSP {worker.lsp_clients}</span>
        <span>Busy chats {worker.busy_chats}</span>
        <span>Exec {worker.exec_running}</span>
      </div>

      {worker.last_error ? (
        <p className={styles.workerError}>{worker.last_error}</p>
      ) : null}
      {error ? <FieldError>{error}</FieldError> : null}

      <div className={styles.actions}>
        {optimistic ? (
          <Button disabled leftIcon={ExternalLink} size="sm" variant="primary">
            Open workspace
          </Button>
        ) : (
          <Button asChild leftIcon={ExternalLink} size="sm" variant="primary">
            <a href={openUrl}>Open workspace</a>
          </Button>
        )}
        <Button
          disabled={pending}
          leftIcon={RefreshCw}
          loading={restartState.isLoading}
          onClick={() => void mutate(() => restart(worker.project_id).unwrap())}
          size="sm"
          variant={
            workerState === "crashed" || workerState === "failed"
              ? "danger"
              : "soft"
          }
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
          variant="ghost"
        >
          Forget
        </Button>
      </div>

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
                })
              }
              variant="danger"
            >
              Forget project
            </Button>
          </div>
        </Dialog.Content>
      </Dialog>
    </Card>
  );
}

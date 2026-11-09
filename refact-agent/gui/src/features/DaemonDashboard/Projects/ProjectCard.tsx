import { useState } from "react";
import {
  ExternalLink,
  MoreHorizontal,
  Pin,
  PinOff,
  RefreshCw,
  Square,
} from "lucide-react";

import {
  Badge,
  Button,
  Card,
  Dialog,
  FieldError,
  IconButton,
  Menu,
  Tooltip,
} from "../../../components/ui";
import {
  useForgetProjectMutation,
  usePinProjectMutation,
  useRestartProjectMutation,
  useStopProjectMutation,
  type DaemonWorker,
} from "../../../services/refact/daemon";
import { useAppDispatch } from "../../../hooks";
import { navigateDashboard } from "../dashboardSlice";
import { workerPresentation, workerStateName } from "./projectRagStatus";
import styles from "./Projects.module.css";

type ProjectCardProps = {
  daemonBase: string;
  worker: DaemonWorker;
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

export function ProjectCard({
  daemonBase,
  worker,
  onMutated,
}: ProjectCardProps) {
  const dispatch = useAppDispatch();
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
          <h3 className={styles.cardTitle}>
            <button
              aria-label={`Open ${worker.slug} details`}
              className={styles.cardTitleButton}
              disabled={optimistic}
              onClick={() =>
                dispatch(
                  navigateDashboard({
                    page: "projects",
                    params: { projectId: worker.project_id },
                  }),
                )
              }
              type="button"
            >
              {worker.slug}
            </button>
          </h3>
          <span className={styles.path} title={worker.root}>
            {worker.root}
          </span>
        </div>
        <Tooltip content={worker.pinned ? "Unpin project" : "Pin project"}>
          <IconButton
            aria-label={
              worker.pinned ? `Unpin ${worker.slug}` : `Pin ${worker.slug}`
            }
            disabled={pending}
            icon={worker.pinned ? PinOff : Pin}
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
            variant="ghost"
          />
        </Tooltip>
      </div>

      <div className={styles.statusRow}>
        <Badge tone={presentation.tone} variant="soft">
          {presentation.label}
        </Badge>
        {workerState === "stopped" ? (
          <span className={styles.muted}>Starts when you open it</span>
        ) : null}
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
          <Button
            className={styles.primaryAction}
            asChild
            leftIcon={ExternalLink}
            size="sm"
            variant="primary"
          >
            <a href={openUrl}>
              {workerState === "stopped" ? "Start & open" : "Open workspace"}
            </a>
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
        <Menu>
          <Menu.Trigger asChild>
            <IconButton
              aria-label={`More actions for ${worker.slug}`}
              disabled={pending}
              icon={MoreHorizontal}
              size="sm"
              variant="ghost"
            />
          </Menu.Trigger>
          <Menu.Content align="end">
            <Menu.Item
              onSelect={() =>
                dispatch(
                  navigateDashboard({
                    page: "activity",
                    params: { projectId: worker.project_id },
                  }),
                )
              }
            >
              View logs
            </Menu.Item>
            <Menu.Item onSelect={() => setForgetOpen(true)}>
              Forget project…
            </Menu.Item>
          </Menu.Content>
        </Menu>
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

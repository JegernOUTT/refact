import { useState } from "react";
import { ChevronDown, ChevronRight, Trash2 } from "lucide-react";

import { Button, Dialog, FieldError } from "../../../components/ui";
import {
  useForgetProjectMutation,
  type DaemonWorker,
} from "../../../services/refact/daemon";
import styles from "./Projects.module.css";

type MissingProjectsSectionProps = {
  workers: DaemonWorker[];
  onMutated: () => void;
};

export function MissingProjectsSection({
  workers,
  onMutated,
}: MissingProjectsSectionProps) {
  const [expanded, setExpanded] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [forgettingId, setForgettingId] = useState<string | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [forget] = useForgetProjectMutation();
  const busy = progress !== null || forgettingId !== null;

  async function forgetOne(projectId: string) {
    setError(null);
    setForgettingId(projectId);
    try {
      await forget(projectId).unwrap();
      onMutated();
    } catch {
      setError("Failed to forget the project.");
    } finally {
      setForgettingId(null);
    }
  }

  async function forgetAllMissing() {
    setError(null);
    setProgress(0);
    try {
      for (const [index, worker] of workers.entries()) {
        setProgress(index);
        await forget(worker.project_id).unwrap();
      }
      setConfirmOpen(false);
      onMutated();
    } catch {
      setError("Failed to forget some projects; the rest were left in place.");
    } finally {
      setProgress(null);
    }
  }

  return (
    <section aria-label="Missing projects" className={styles.missingSection}>
      <div className={styles.missingHeader}>
        <button
          aria-expanded={expanded}
          className={styles.missingToggle}
          onClick={() => setExpanded((value) => !value)}
          type="button"
        >
          {expanded ? (
            <ChevronDown aria-hidden size={14} />
          ) : (
            <ChevronRight aria-hidden size={14} />
          )}
          Missing projects ({workers.length})
        </button>
        <Button
          disabled={busy}
          leftIcon={Trash2}
          onClick={() => setConfirmOpen(true)}
          size="sm"
          variant="ghost"
        >
          Forget all missing
        </Button>
      </div>

      {error && !confirmOpen ? <FieldError>{error}</FieldError> : null}

      {expanded ? (
        <ul className={styles.missingList}>
          {workers.map((worker) => (
            <li className={styles.missingRow} key={worker.project_id}>
              <span className={styles.missingName}>{worker.slug}</span>
              <span className={styles.missingPath} title={worker.root}>
                {worker.root}
              </span>
              <Button
                aria-label={`Forget ${worker.slug}`}
                disabled={busy}
                loading={forgettingId === worker.project_id}
                onClick={() => void forgetOne(worker.project_id)}
                size="sm"
                variant="ghost"
              >
                Forget
              </Button>
            </li>
          ))}
        </ul>
      ) : null}

      <Dialog
        open={confirmOpen}
        onOpenChange={(open) => {
          if (!busy) setConfirmOpen(open);
        }}
      >
        <Dialog.Content maxWidth="calc(var(--rf-space-6) * 12)">
          <Dialog.Title>
            Forget {workers.length} missing project
            {workers.length === 1 ? "" : "s"}?
          </Dialog.Title>
          <Dialog.Description>
            This removes {workers.length} project
            {workers.length === 1 ? "" : "s"} whose folders no longer exist from
            the dashboard.
          </Dialog.Description>
          {error ? <FieldError>{error}</FieldError> : null}
          <div className={styles.dialogActions}>
            <Dialog.Close asChild>
              <Button disabled={busy} variant="soft">
                Cancel
              </Button>
            </Dialog.Close>
            <Button
              disabled={busy}
              loading={progress !== null}
              onClick={() => void forgetAllMissing()}
              variant="danger"
            >
              {progress !== null
                ? `Forgetting ${progress + 1} of ${workers.length}…`
                : `Forget ${workers.length} project${
                    workers.length === 1 ? "" : "s"
                  }`}
            </Button>
          </div>
        </Dialog.Content>
      </Dialog>
    </section>
  );
}

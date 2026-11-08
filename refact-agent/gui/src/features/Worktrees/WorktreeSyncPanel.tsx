import { TriangleAlert } from "lucide-react";
import React, { useCallback, useMemo, useState } from "react";
import { FieldSelect } from "../../components/ui";
import {
  KitBadge,
  KitButton,
  KitCheckRow,
  KitDialog,
  KitDialogCancel,
  KitDivider,
  KitSection,
  KitStaticRow,
  type KitTone,
} from "../../components/ComposerKit";
import type { WorktreeSyncController } from "../../hooks/useWorktreeActions";
import type {
  WorktreeSyncConflict,
  WorktreeSyncResolution,
  WorktreeSyncResolutionChoice,
  WorktreeSyncStatus,
} from "../../services/refact";
import { humanizeIdentifier } from "../../utils/displayNames";
import type { WorktreeWriterBlocker } from "../../services/refact/worktrees";
import styles from "./Worktrees.module.css";

type WorktreeSyncPanelProps = {
  controller: WorktreeSyncController;
  disabled?: boolean;
  onAskRefact?: () => void | Promise<void>;
};

const STATUS_LABELS: Record<WorktreeSyncStatus, string> = {
  idle: "No synchronization",
  conflicts: "Conflicts to resolve",
  ready: "Ready to continue",
  completed: "Synchronized",
  aborted: "Aborted",
  recovery_required: "Recovery required",
  target_advanced: "Target advanced",
};

const STATUS_TONES: Record<WorktreeSyncStatus, KitTone> = {
  idle: "muted",
  conflicts: "warning",
  ready: "success",
  completed: "success",
  aborted: "muted",
  recovery_required: "danger",
  target_advanced: "warning",
};

const CHOICE_LABELS: Record<WorktreeSyncResolutionChoice, string> = {
  working_tree: "Keep edited working file",
  source: "Use source version",
  target: "Use target version",
  delete: "Delete file",
};

function shortHead(head?: string | null): string {
  return head ? head.slice(0, 8) : "unknown";
}

function choiceOptions(conflict: WorktreeSyncConflict) {
  const choices: WorktreeSyncResolutionChoice[] = ["working_tree"];
  if (conflict.source) choices.push("source");
  if (conflict.target) choices.push("target");
  choices.push("delete");
  return choices.map((choice) => ({
    value: choice,
    label: CHOICE_LABELS[choice],
  }));
}

type PendingConfirmation = "abort" | "restore" | "recover";

const WRITER_ACTION_LABELS: Record<
  WorktreeWriterBlocker["recovery_actions"][number],
  string
> = {
  wait: "Wait for this writer to finish",
  inspect_agent: "Ask the owning chat to inspect this agent",
  cancel_agent: "Ask the owning chat to cancel this agent if authorized",
  stop_chat: "Stop the owning chat before retrying",
  user_required: "User action is required",
  retry_status: "Refresh status after resolving the blocker",
};

type KeyedState<T> = { key: string; value: T };

export const WorktreeSyncPanel: React.FC<WorktreeSyncPanelProps> = ({
  controller,
  disabled,
  onAskRefact,
}) => {
  const {
    current,
    isLoading,
    isActive,
    canRestore,
    busy,
    error,
    sourceDirty,
    checkpointConsent,
    setCheckpointConsent,
    statusError,
    statusUnavailable,
  } = controller;
  const operation = current?.operation ?? null;
  const scopeKey = `${current?.id ?? ""}:${operation?.operation_id ?? ""}`;
  const [choiceState, setChoiceState] = useState<
    KeyedState<Partial<Record<string, WorktreeSyncResolutionChoice>>>
  >({ key: scopeKey, value: {} });
  const [confirmState, setConfirmState] = useState<
    KeyedState<PendingConfirmation | null>
  >({ key: scopeKey, value: null });
  const [forceState, setForceState] = useState<KeyedState<boolean>>({
    key: scopeKey,
    value: false,
  });
  const choices = useMemo<
    Partial<Record<string, WorktreeSyncResolutionChoice>>
  >(
    () => (choiceState.key === scopeKey ? choiceState.value : {}),
    [choiceState, scopeKey],
  );
  const confirm = confirmState.key === scopeKey ? confirmState.value : null;
  const forceAcknowledged =
    forceState.key === scopeKey ? forceState.value : false;

  const [archiveState, setArchiveState] = useState<KeyedState<string>>({
    key: scopeKey,
    value: "primary",
  });
  const archiveId =
    operation?.phase === "restoring"
      ? current?.restore_archive_id ?? "primary"
      : archiveState.key === scopeKey
        ? archiveState.value
        : "primary";
  const archives = current?.recovery_archives ?? [];
  const selectedArchive = archives.find(
    (archive) => archive.archive_id === archiveId,
  );
  const conflicts = useMemo(() => current?.conflicts ?? [], [current]);
  const recoveryDigest = current?.recovery_journal_digest ?? null;
  const status = current?.status ?? "idle";
  const statusStale = statusError !== null;
  const writers = current?.writer_readiness;
  const writersBlocked = writers?.ready === false;
  const controlsDisabled =
    busy || disabled === true || statusStale || writersBlocked;

  const pendingResolutions = useMemo(
    () =>
      conflicts.flatMap((conflict): WorktreeSyncResolution[] => {
        const choice = choices[conflict.path];
        return choice ? [{ path: conflict.path, choice }] : [];
      }),
    [choices, conflicts],
  );

  const canContinue =
    isActive &&
    conflicts.length === 0 &&
    (status === "ready" || status === "target_advanced");
  const needsRefreshSync = !isActive && status === "target_advanced";
  const showStart = !isActive && status !== "recovery_required";
  const canRecover =
    status === "recovery_required" &&
    Boolean(recoveryDigest) &&
    (current?.warnings.some((warning) => /journal|metadata/i.test(warning)) ??
      false);

  const setConfirm = useCallback(
    (value: PendingConfirmation | null) =>
      setConfirmState({ key: scopeKey, value }),
    [scopeKey],
  );
  const closeConfirm = useCallback(() => {
    setConfirmState({ key: scopeKey, value: null });
    setForceState({ key: scopeKey, value: false });
  }, [scopeKey]);

  const handleResolve = useCallback(async () => {
    const result = await controller.resolve(pendingResolutions);
    if (result) setChoiceState({ key: scopeKey, value: {} });
  }, [controller, pendingResolutions, scopeKey]);

  const handleAbort = useCallback(
    async (force: boolean) => {
      const result = await controller.abort(force);
      if (result) closeConfirm();
    },
    [closeConfirm, controller],
  );

  const handleRestore = useCallback(async () => {
    const result = await controller.restore(
      archives.length > 0 ? archiveId : undefined,
    );
    if (result) closeConfirm();
  }, [archiveId, archives.length, closeConfirm, controller]);

  const handleRecover = useCallback(async () => {
    const result = await controller.recover();
    if (result) closeConfirm();
  }, [closeConfirm, controller]);

  if (isLoading && !current) {
    return <KitStaticRow tone="muted" label="Loading synchronization…" />;
  }

  if (statusUnavailable) {
    return (
      <section aria-label="Source synchronization">
        <KitSection>Source synchronization</KitSection>
        <p className={styles.formError} role="alert">
          <TriangleAlert aria-hidden="true" />
          {statusError ?? "Synchronization status is unavailable."}
        </p>
        <div className={styles.inlineActions}>
          <KitButton
            disabled={busy || disabled === true}
            onClick={() => void controller.refresh()}
          >
            Refresh status
          </KitButton>
        </div>
      </section>
    );
  }

  return (
    <section aria-label="Source synchronization">
      <KitSection>Source synchronization</KitSection>
      {writers ? (
        <>
          <KitStaticRow
            label={writers.ready ? "Writers ready" : "Waiting for writers"}
            tone="muted"
          />
          {writers.blockers.map((blocker, index) => (
            <KitStaticRow
              key={`${blocker.kind}-${
                blocker.agent_id ?? blocker.chat_id ?? index
              }`}
              label={humanizeIdentifier(blocker.kind)}
              sub={[
                blocker.agent_id && `Agent ${blocker.agent_id}`,
                blocker.chat_id && `Chat ${blocker.chat_id}`,
                blocker.run_generation !== undefined &&
                  `Run ${blocker.run_generation}`,
                ...blocker.recovery_actions.map(
                  (action) => WRITER_ACTION_LABELS[action],
                ),
              ]
                .filter(Boolean)
                .join(" · ")}
            />
          ))}
          {writersBlocked ? (
            <p className={`${styles.warningText} ${styles.resultLine}`}>
              Ask the owning chat to inspect or wait for the listed writers, or
              request an authorized stop, then refresh status. Nothing is
              stopped or retried automatically.
            </p>
          ) : null}
          {writers.truncated ? (
            <p className={`${styles.warningText} ${styles.resultLine}`}>
              More writers may be blocking. Refresh status after resolving the
              listed writers.
            </p>
          ) : null}
          {writersBlocked && current.instructions ? (
            <p className={`${styles.metaText} ${styles.resultLine}`}>
              {current.instructions}
            </p>
          ) : null}
        </>
      ) : null}
      <KitStaticRow
        label={STATUS_LABELS[status]}
        sub={
          operation
            ? `Operation ${operation.operation_id} · revision ${
                operation.revision
              } · ${humanizeIdentifier(operation.phase)}`
            : undefined
        }
        badges={
          <KitBadge tone={STATUS_TONES[status]}>
            {STATUS_LABELS[status]}
          </KitBadge>
        }
      />
      {current ? (
        <p className={`${styles.metaText} ${styles.resultLine}`}>
          Source {shortHead(operation?.source_head ?? current.source_head)}
          {" → "}
          {operation?.target_branch ?? current.target_branch ?? "target"}{" "}
          {shortHead(operation?.target_head ?? current.target_head)}
        </p>
      ) : null}
      {operation?.checkpoint_commit ? (
        <p className={`${styles.metaText} ${styles.resultLine}`}>
          Checkpoint commit: {shortHead(operation.checkpoint_commit)}
        </p>
      ) : null}
      {operation?.recovery_path ? (
        <p className={`${styles.metaText} ${styles.resultLine}`}>
          Recovery backup: {operation.recovery_path}
        </p>
      ) : null}
      {archives.length > 0 ? (
        <select
          aria-label="Recovery backup"
          value={archiveId}
          disabled={controlsDisabled || operation?.phase === "restoring"}
          onChange={(event) =>
            setArchiveState({ key: scopeKey, value: event.target.value })
          }
        >
          {archives.map((archive) => (
            <option key={archive.archive_id} value={archive.archive_id}>
              {archive.archive_id}: {archive.path}
            </option>
          ))}
        </select>
      ) : null}
      {status === "target_advanced" ? (
        <p className={`${styles.warningText} ${styles.resultLine}`}>
          {isActive
            ? "The target branch moved after synchronization started. Finish this operation, then synchronize again against the fresh target before landing."
            : "The target branch moved since the last synchronization. Synchronize again before landing."}
        </p>
      ) : null}
      {status === "recovery_required" ? (
        <p className={`${styles.warningText} ${styles.resultLine}`}>
          Synchronization needs recovery. Refresh the status; nothing is
          replayed automatically.
        </p>
      ) : null}
      {current?.warnings.map((warning, index) => (
        <p
          key={`${index}-${warning}`}
          className={`${styles.warningText} ${styles.resultLine}`}
        >
          {warning}
        </p>
      ))}

      {conflicts.length > 0 ? (
        <>
          <KitSection>Conflicted files</KitSection>
          {conflicts.map((conflict) => (
            <div key={conflict.path} className={styles.field}>
              <span className={`${styles.labelText} ${styles.resultLine}`}>
                {conflict.path}
              </span>
              <span className={styles.fieldControl}>
                <FieldSelect
                  aria-label={`Resolution for ${conflict.path}`}
                  placeholder="Choose resolution"
                  value={choices[conflict.path] ?? ""}
                  options={choiceOptions(conflict)}
                  disabled={controlsDisabled}
                  onChange={(value) =>
                    setChoiceState({
                      key: scopeKey,
                      value: {
                        ...choices,
                        [conflict.path]: value as WorktreeSyncResolutionChoice,
                      },
                    })
                  }
                />
              </span>
            </div>
          ))}
          <div className={styles.inlineActions}>
            <KitButton
              variant="primary"
              disabled={controlsDisabled || pendingResolutions.length === 0}
              onClick={() => void handleResolve()}
            >
              Apply resolutions
            </KitButton>
          </div>
        </>
      ) : null}

      {showStart && sourceDirty ? (
        <KitCheckRow
          label="Checkpoint uncommitted source changes"
          hint="Commits them on the source branch first so they are preserved"
          ariaLabel="Confirm checkpointing uncommitted source changes"
          checked={checkpointConsent}
          onCheckedChange={setCheckpointConsent}
          disabled={controlsDisabled}
        />
      ) : null}

      <div className={styles.inlineActions}>
        {showStart ? (
          <KitButton
            variant="primary"
            disabled={controlsDisabled || (sourceDirty && !checkpointConsent)}
            onClick={() => void controller.start()}
          >
            {needsRefreshSync ? "Synchronize again" : "Synchronize with target"}
          </KitButton>
        ) : null}
        {canContinue ? (
          <KitButton
            variant="primary"
            disabled={controlsDisabled}
            onClick={() => void controller.continueSync()}
          >
            Continue synchronization
          </KitButton>
        ) : null}
        {isActive ? (
          <KitButton
            variant="danger"
            disabled={controlsDisabled}
            onClick={() => setConfirm("abort")}
          >
            Abort…
          </KitButton>
        ) : null}
        {canRestore ? (
          <KitButton
            disabled={controlsDisabled}
            onClick={() => setConfirm("restore")}
          >
            Restore backup…
          </KitButton>
        ) : null}
        {canRecover ? (
          <KitButton
            variant="danger"
            disabled={controlsDisabled}
            onClick={() => setConfirm("recover")}
          >
            Recover journal…
          </KitButton>
        ) : null}
        {isActive && onAskRefact ? (
          <KitButton
            disabled={busy || disabled === true || statusStale}
            onClick={() => void onAskRefact()}
          >
            Ask Refact to help
          </KitButton>
        ) : null}
        <KitButton
          disabled={busy || disabled === true}
          onClick={() => void controller.refresh()}
        >
          Refresh status
        </KitButton>
      </div>

      {statusError ? (
        <p className={styles.formError} role="alert">
          <TriangleAlert aria-hidden="true" />
          {statusError} Showing the last known state; refresh before acting.
        </p>
      ) : null}
      {error ? (
        <p className={styles.formError} role="alert">
          <TriangleAlert aria-hidden="true" />
          {error}
        </p>
      ) : null}
      <KitDivider />

      <KitDialog
        open={confirm === "abort"}
        onOpenChange={(open) => {
          if (!open) closeConfirm();
        }}
        title="Abort synchronization"
        description="Stop this synchronization and return the source to its pre-merge state."
        width="480px"
        footer={
          <>
            <KitDialogCancel disabled={busy}>
              Keep synchronizing
            </KitDialogCancel>
            <KitButton
              disabled={controlsDisabled}
              onClick={() => void handleAbort(false)}
            >
              Abort safely
            </KitButton>
            <KitButton
              variant="danger"
              disabled={controlsDisabled || !forceAcknowledged}
              onClick={() => void handleAbort(true)}
            >
              Force abort with backup
            </KitButton>
          </>
        }
      >
        <p className={`${styles.metaText} ${styles.resultLine}`}>
          A safe abort refuses when you have edited the integration. A forced
          abort writes a durable backup first, then resets the source.
        </p>
        <KitCheckRow
          label="I understand edits will be reset after a backup is written"
          ariaLabel="Acknowledge forced abort resets the source after backup"
          checked={forceAcknowledged}
          onCheckedChange={(checked) =>
            setForceState({ key: scopeKey, value: checked })
          }
          disabled={busy}
        />
        {error ? (
          <p className={styles.formError} role="alert">
            <TriangleAlert aria-hidden="true" />
            {error}
          </p>
        ) : null}
      </KitDialog>

      <KitDialog
        open={confirm === "restore"}
        onOpenChange={(open) => {
          if (!open) closeConfirm();
        }}
        title="Restore backup"
        description="Re-apply the backed-up integration state to the source."
        width="480px"
        footer={
          <>
            <KitDialogCancel disabled={busy}>Cancel</KitDialogCancel>
            <KitButton
              variant="primary"
              disabled={controlsDisabled}
              onClick={() => void handleRestore()}
            >
              Restore backup
            </KitButton>
          </>
        }
      >
        <p className={`${styles.metaText} ${styles.resultLine}`}>
          Restore refuses to overwrite work done after the abort. Backup:{" "}
          {selectedArchive?.path ?? operation?.recovery_path}
        </p>
        {error ? (
          <p className={styles.formError} role="alert">
            <TriangleAlert aria-hidden="true" />
            {error}
          </p>
        ) : null}
      </KitDialog>

      <KitDialog
        open={confirm === "recover"}
        onOpenChange={(open) => {
          if (!open) closeConfirm();
        }}
        title="Recover synchronization journal"
        description="Archive the synchronization metadata identified by the recovery receipt."
        width="480px"
        footer={
          <>
            <KitDialogCancel disabled={busy}>Cancel</KitDialogCancel>
            <KitButton
              variant="danger"
              disabled={controlsDisabled}
              onClick={() => void handleRecover()}
            >
              Archive journal and recover
            </KitButton>
          </>
        }
      >
        <p className={`${styles.metaText} ${styles.resultLine}`}>
          This archives synchronization metadata and may restore a validated
          journal backup. No repository files or commits are changed or
          discarded; unknown Git state is retained.
        </p>
        {error ? (
          <p className={styles.formError} role="alert">
            <TriangleAlert aria-hidden="true" />
            {error}
          </p>
        ) : null}
      </KitDialog>
    </section>
  );
};

WorktreeSyncPanel.displayName = "WorktreeSyncPanel";

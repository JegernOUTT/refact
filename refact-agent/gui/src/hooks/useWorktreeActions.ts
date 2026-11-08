import { useCallback, useEffect, useRef, useState } from "react";
import {
  useGetWorktreeSyncStatusQuery,
  useSyncWorktreeMutation,
  type WorktreeSyncRequest,
  type WorktreeSyncResolution,
  type WorktreeSyncResponse,
} from "../services/refact";
import { worktreeErrorText } from "../features/Worktrees/worktreeError";

export const WORKTREE_SYNC_TOOL = "worktree_sync";

const TERMINAL_PHASES = new Set([
  "completed",
  "aborted",
  "restored",
  "metadata_archived",
]);

const noop = () => undefined;

class StaleSourceError extends Error {}

export function isSyncOperationActive(
  response?: WorktreeSyncResponse | null,
): boolean {
  const operation = response?.operation;
  if (!operation) return false;
  if (
    response.status === "recovery_required" &&
    operation.phase === "restoring"
  )
    return true;
  return (
    !["idle", "completed", "aborted"].includes(response.status) &&
    !TERMINAL_PHASES.has(operation.phase)
  );
}

export function canRestoreSyncBackup(
  response?: WorktreeSyncResponse | null,
): boolean {
  const operation = response?.operation;
  return (
    (operation?.phase === "aborted" || operation?.phase === "restoring") &&
    Boolean(operation.recovery_path)
  );
}

type UseWorktreeSyncArgs = {
  worktreeId?: string | null;
  sourceWorkspaceRoot?: string;
  targetBranch?: string;
  enabled?: boolean;
  sourceDirty?: boolean;
};

type PostSync = (request: WorktreeSyncRequest) => Promise<WorktreeSyncResponse>;

type PublishedResponse = {
  key: string;
  at: number;
  response: WorktreeSyncResponse;
};

export function useWorktreeSync({
  worktreeId,
  sourceWorkspaceRoot,
  targetBranch,
  enabled = true,
  sourceDirty = false,
}: UseWorktreeSyncArgs) {
  const id = worktreeId ?? "";
  const skip = !enabled || id.length === 0;
  const trimmedTarget = targetBranch?.trim();
  const requestedTarget = trimmedTarget === "" ? undefined : trimmedTarget;
  const sourceKey = JSON.stringify([
    id,
    sourceWorkspaceRoot ?? null,
    requestedTarget ?? null,
  ]);
  const sourceKeyRef = useRef(sourceKey);
  sourceKeyRef.current = sourceKey;
  const lifecycleKey = JSON.stringify([sourceKey, enabled]);
  const lifecycle = useRef({ key: lifecycleKey, generation: 0 });
  useEffect(
    () => () => {
      lifecycle.current.generation += 1;
    },
    [],
  );
  if (lifecycle.current.key !== lifecycleKey) {
    lifecycle.current = {
      key: lifecycleKey,
      generation: lifecycle.current.generation + 1,
    };
  }

  const statusQuery = useGetWorktreeSyncStatusQuery(
    {
      id,
      source_workspace_root: sourceWorkspaceRoot,
      target_branch: requestedTarget,
    },
    { skip },
  );
  const [syncWorktree] = useSyncWorktreeMutation();
  const [published, setPublished] = useState<PublishedResponse | null>(null);
  const [errorState, setErrorState] = useState<{
    key: string;
    message: string;
  } | null>(null);
  const [busyOwner, setBusyOwner] = useState<{
    key: string;
    generation: number;
    token: number;
  } | null>(null);
  const [consentKey, setConsentKey] = useState<string | null>(null);
  const busyRef = useRef<typeof busyOwner>(null);
  const runSequence = useRef(0);
  const refetchRef = useRef<() => void>(noop);
  refetchRef.current = skip ? noop : () => void statusQuery.refetch();

  const queryData = skip ? null : statusQuery.currentData ?? null;
  const queryAt = statusQuery.fulfilledTimeStamp ?? 0;
  const latest =
    published !== null && published.key === sourceKey && published.at >= queryAt
      ? published.response
      : null;
  const current = skip ? null : latest ?? queryData;
  const currentRef = useRef(current);
  currentRef.current = current;

  useEffect(() => {
    if (published !== null && published.key !== sourceKey) setPublished(null);
  }, [published, sourceKey]);

  const busy =
    busyOwner?.key === sourceKey &&
    busyOwner.generation === lifecycle.current.generation;
  const error = errorState?.key === sourceKey ? errorState.message : null;
  const checkpointConsent = consentKey === sourceKey;
  const setCheckpointConsent = useCallback(
    (value: boolean) => setConsentKey(value ? sourceKeyRef.current : null),
    [],
  );

  const run = useCallback(
    async (
      operation: (post: PostSync) => Promise<WorktreeSyncResponse>,
    ): Promise<WorktreeSyncResponse | null> => {
      const key = sourceKey;
      const generation = lifecycle.current.generation;
      if (
        busyRef.current?.key === key &&
        busyRef.current.generation === generation
      )
        return null;
      const owner = { key, generation, token: ++runSequence.current };
      const assertCurrent = () => {
        if (
          sourceKeyRef.current !== key ||
          lifecycle.current.generation !== generation
        )
          throw new StaleSourceError();
      };
      const post: PostSync = async (request) => {
        assertCurrent();
        const response = await syncWorktree({
          id,
          source_workspace_root: sourceWorkspaceRoot,
          request,
        }).unwrap();
        assertCurrent();
        setPublished({
          key,
          at: Date.now(),
          response: {
            ...response,
            writer_readiness:
              response.writer_readiness ?? currentRef.current?.writer_readiness,
          },
        });
        return response;
      };
      busyRef.current = owner;
      setBusyOwner(owner);
      setErrorState(null);
      try {
        return await operation(post);
      } catch (caught) {
        if (
          caught instanceof StaleSourceError ||
          sourceKeyRef.current !== key ||
          lifecycle.current.generation !== generation
        ) {
          return null;
        }
        setPublished(null);
        setErrorState({ key, message: worktreeErrorText(caught) });
        refetchRef.current();
        return null;
      } finally {
        if (busyRef.current === owner) busyRef.current = null;
        setBusyOwner((previous) =>
          previous?.token === owner.token ? null : previous,
        );
      }
    },
    [id, sourceKey, sourceWorkspaceRoot, syncWorktree],
  );

  const start = useCallback(
    () =>
      run(async (post) => {
        const status = await post({
          action: "status",
          target_branch: requestedTarget,
        });
        if (isSyncOperationActive(status)) return status;
        if (status.writer_readiness?.ready === false) return status;
        if (status.status === "recovery_required")
          throw new Error(
            "Recover synchronization before starting a new operation.",
          );
        if (sourceDirty && !checkpointConsent) {
          throw new Error(
            "Source has uncommitted changes. Confirm checkpointing them before starting synchronization.",
          );
        }
        if (
          !status.target_branch ||
          !status.target_head ||
          !status.source_head ||
          (requestedTarget && status.target_branch !== requestedTarget)
        ) {
          throw new Error(
            "Pinned source and target heads are unavailable; refresh status before starting synchronization.",
          );
        }
        return post({
          action: "start",
          target_branch: status.target_branch,
          expected_source_head: status.source_head,
          expected_target_head: status.target_head,
          checkpoint: sourceDirty && checkpointConsent,
        });
      }),
    [checkpointConsent, requestedTarget, run, sourceDirty],
  );

  const refresh = useCallback(async () => {
    const result = await run((post) =>
      post({ action: "status", target_branch: requestedTarget }),
    );
    refetchRef.current();
    return result;
  }, [requestedTarget, run]);

  const withOperation = useCallback(
    (
      build: (operation: {
        operation_id: string;
        expected_revision: number;
      }) => WorktreeSyncRequest,
    ) =>
      run((post) => {
        const operation = currentRef.current?.operation;
        if (!operation) {
          throw new Error("No synchronization operation is available.");
        }
        return post(
          build({
            operation_id: operation.operation_id,
            expected_revision: operation.revision,
          }),
        );
      }),
    [run],
  );

  const resolve = useCallback(
    (resolutions: WorktreeSyncResolution[]) =>
      withOperation((operation) => ({
        action: "resolve",
        ...operation,
        resolutions,
      })),
    [withOperation],
  );
  const continueSync = useCallback(
    (commitMessage?: string) =>
      withOperation((operation) => ({
        action: "continue",
        ...operation,
        ...(commitMessage && commitMessage.trim().length > 0
          ? { commit_message: commitMessage.trim() }
          : {}),
      })),
    [withOperation],
  );
  const abort = useCallback(
    (force: boolean) =>
      withOperation((operation) => ({
        action: "abort",
        ...operation,
        force,
      })),
    [withOperation],
  );
  const restore = useCallback(
    (archiveId?: string) =>
      withOperation((operation) => ({
        action: "restore",
        ...operation,
        ...(archiveId ? { archive_id: archiveId } : {}),
      })),
    [withOperation],
  );

  const recover = useCallback(
    () =>
      run((post) => {
        const digest = currentRef.current?.recovery_journal_digest;
        if (!digest) {
          throw new Error("No recovery receipt is available.");
        }
        return post({ action: "recover", expected_journal_digest: digest });
      }),
    [run],
  );

  const isToolAvailable = useCallback(
    (): Promise<boolean | undefined> => Promise.resolve(undefined),
    [],
  );

  const statusError =
    !skip && statusQuery.isError ? worktreeErrorText(statusQuery.error) : null;

  return {
    current,
    statusError,
    statusUnavailable: !skip && current === null && !statusQuery.isLoading,
    isLoading: !skip && statusQuery.isLoading,
    isActive: isSyncOperationActive(current),
    targetMismatch:
      isSyncOperationActive(current) &&
      Boolean(
        requestedTarget &&
          current?.operation?.target_branch !== requestedTarget,
      ),
    canRestore: canRestoreSyncBackup(current),
    busy,
    error,
    sourceDirty,
    checkpointConsent,
    setCheckpointConsent,
    start,
    refresh,
    resolve,
    continueSync,
    abort,
    restore,
    recover,
    isToolAvailable,
  };
}

export type WorktreeSyncController = ReturnType<typeof useWorktreeSync>;

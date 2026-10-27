import { useCallback, useMemo, useState, type ReactNode } from "react";

import { useCopyToClipboard } from "../../../../hooks/useCopyToClipboard";
import {
  useCheckoutGitRefMutation,
  useCreateGitBranchMutation,
  useDeleteGitBranchMutation,
  useFetchGitRemoteMutation,
  useGetGitBranchesQuery,
  usePullGitRemoteMutation,
  usePushGitRemoteMutation,
  useRenameGitBranchMutation,
  type GitBranchesRoot,
  type GitCheckoutKind,
  type GitCommitLogEntry,
  type NormalizedGitStatus,
} from "../../../../services/refact/gitRead";
import { gitErrorCode, gitErrorText, type GitErrorView } from "../gitErrors";
import type { BranchRef } from "../log/branchesTreeModel";
import { repoBlockedReason } from "../repoState";
import {
  CheckoutRevisionDialog,
  DeleteBranchDialog,
  NewBranchDialog,
  RenameBranchDialog,
} from "./BranchDialogs";
import { remoteOfRef } from "./branchName";
import type { GitFeedbackState } from "./GitFeedback";
import type { GitMenuAction } from "./menuActions";
import { PushDialog, type PushTarget } from "./PushDialog";

const SHORT_OID_LENGTH = 8;

type DialogState =
  | {
      kind: "new";
      startPoint: string | null;
      startLabel: string;
      defaultName: string;
    }
  | { kind: "rename"; name: string }
  | { kind: "delete"; name: string; notMerged: boolean }
  | { kind: "revision" }
  | { kind: "push"; setUpstream: boolean; mismatch?: boolean };

export type NewBranchSource = {
  startPoint: string | null;
  label: string;
  defaultName?: string;
};

function plural(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

function remoteNames(branches: GitBranchesRoot | undefined): string[] {
  const names = new Set(branches?.remotes ?? []);
  for (const branch of branches?.remote_branches ?? [])
    names.add(branch.remote);
  return [...names];
}

export type UseGitActionsOptions = {
  root: string;
  status: NormalizedGitStatus | null;
};

export function useGitActions({ root, status }: UseGitActionsOptions) {
  const copy = useCopyToClipboard();
  const branchesQuery = useGetGitBranchesQuery(root, { skip: !root });
  const branches = branchesQuery.data?.roots.at(0);
  const [checkoutRef, checkoutState] = useCheckoutGitRefMutation();
  const [createBranch, createState] = useCreateGitBranchMutation();
  const [renameBranch, renameState] = useRenameGitBranchMutation();
  const [deleteBranch, deleteState] = useDeleteGitBranchMutation();
  const [fetchRemote, fetchState] = useFetchGitRemoteMutation();
  const [pullRemote, pullState] = usePullGitRemoteMutation();
  const [pushRemote, pushState] = usePushGitRemoteMutation();
  const [dialog, setDialog] = useState<DialogState | null>(null);
  const [dialogError, setDialogError] = useState<GitErrorView | null>(null);
  const [feedback, setFeedback] = useState<GitFeedbackState | null>(null);

  const blockedReason = repoBlockedReason(status?.repoState);
  const currentBranch = status?.branch ?? branches?.current ?? null;
  const updateBlockedReason =
    blockedReason ??
    (status?.headDetached
      ? "HEAD is detached — check out a branch to update the project."
      : status?.extended && currentBranch && !status.upstream
        ? `${currentBranch} has no upstream branch — push it with Push… to set one.`
        : null);
  const localNames = useMemo(
    () => (branches?.branches ?? []).map((branch) => branch.name),
    [branches?.branches],
  );
  const remotes = useMemo(() => remoteNames(branches), [branches]);
  const tags = useMemo(
    () => (branches?.tags ?? []).map((tag) => tag.name),
    [branches?.tags],
  );

  const reportError = useCallback(
    (error: unknown) =>
      setFeedback({ tone: "error", error: gitErrorText(error) }),
    [],
  );

  const openDialog = useCallback((next: DialogState) => {
    setDialogError(null);
    setDialog(next);
  }, []);

  const closeDialog = useCallback(() => {
    setDialog(null);
    setDialogError(null);
  }, []);

  const checkout = useCallback(
    async (name: string, kind: GitCheckoutKind): Promise<boolean> => {
      if (blockedReason) {
        setFeedback({
          tone: "error",
          error: {
            code: null,
            message: blockedReason,
            output: null,
            files: [],
          },
        });
        return false;
      }
      setFeedback(null);
      try {
        const response = await checkoutRef({ root, name, kind }).unwrap();
        const target =
          response.branch ??
          (response.head_oid
            ? response.head_oid.slice(0, SHORT_OID_LENGTH)
            : name);
        setFeedback({
          tone: "success",
          text: response.detached
            ? `Checked out ${target} (detached HEAD)`
            : `Checked out ${target}`,
        });
        return true;
      } catch (error) {
        reportError(error);
        return false;
      }
    },
    [blockedReason, checkoutRef, reportError, root],
  );

  const checkoutBranchRef = useCallback(
    (ref: BranchRef) => {
      if (ref.current) return;
      void checkout(ref.name, ref.kind);
    },
    [checkout],
  );

  const openNewBranch = useCallback(
    (source?: NewBranchSource) => {
      const headLabel =
        currentBranch ??
        (status?.headOid ? status.headOid.slice(0, SHORT_OID_LENGTH) : "HEAD");
      openDialog({
        kind: "new",
        startPoint: source?.startPoint ?? null,
        startLabel: source?.label ?? headLabel,
        defaultName: source?.defaultName ?? "",
      });
    },
    [currentBranch, openDialog, status?.headOid],
  );

  const openNewBranchFromRef = useCallback(
    (ref: BranchRef) => {
      const tracked =
        ref.kind === "remote" ? remoteOfRef(ref.name, remotes) : null;
      openNewBranch({
        startPoint: ref.name,
        label: `'${ref.name}'`,
        defaultName: tracked?.branch ?? "",
      });
    },
    [openNewBranch, remotes],
  );

  const openPush = useCallback(
    (setUpstream = false) => {
      if (!currentBranch || status?.headDetached) return;
      openDialog({ kind: "push", setUpstream });
    },
    [currentBranch, openDialog, status?.headDetached],
  );

  const fetch = useCallback(async () => {
    setFeedback(null);
    try {
      const response = await fetchRemote({ root }).unwrap();
      setFeedback({ tone: "success", text: response.summary || "Fetched" });
    } catch (error) {
      reportError(error);
    }
  }, [fetchRemote, reportError, root]);

  const pull = useCallback(async () => {
    if (updateBlockedReason) {
      setFeedback({
        tone: "error",
        error: {
          code: null,
          message: updateBlockedReason,
          output: null,
          files: [],
        },
      });
      return;
    }
    setFeedback(null);
    try {
      const response = await pullRemote({ root }).unwrap();
      setFeedback({ tone: "success", text: response.summary || "Updated" });
    } catch (error) {
      reportError(error);
    }
  }, [pullRemote, reportError, root, updateBlockedReason]);

  const push = async (target: PushTarget) => {
    if (!currentBranch) return;
    setDialogError(null);
    try {
      const response = await pushRemote({
        root,
        remote: target.remote,
        branch: currentBranch,
        ...(target.setUpstream ? { set_upstream: true } : {}),
        ...(target.forceWithLease ? { force_with_lease: true } : {}),
      }).unwrap();
      closeDialog();
      const count = status?.ahead ?? 0;
      setFeedback({
        tone: "success",
        text:
          response.summary ||
          `Pushed ${plural(count, "commit")} to ${target.destination}`,
      });
    } catch (error) {
      const code = gitErrorCode(error);
      if (code === "no_upstream" || code === "upstream_mismatch") {
        // Re-open with an explicit target the user picks (remote/branch).
        openDialog({
          kind: "push",
          setUpstream: true,
          mismatch: code === "upstream_mismatch",
        });
        setDialogError(gitErrorText(error));
        return;
      }
      setDialogError(gitErrorText(error));
    }
  };

  const create = async (
    name: string,
    checkoutAfter: boolean,
    startPoint: string | null,
  ) => {
    setDialogError(null);
    try {
      const response = await createBranch({
        root,
        name,
        checkout: checkoutAfter,
        ...(startPoint ? { start_point: startPoint } : {}),
      }).unwrap();
      closeDialog();
      setFeedback({
        tone: "success",
        text: response.checked_out
          ? `Created and checked out ${response.name}`
          : `Created branch ${response.name}`,
      });
    } catch (error) {
      setDialogError(gitErrorText(error));
    }
  };

  const rename = async (oldName: string, newName: string) => {
    setDialogError(null);
    try {
      const response = await renameBranch({
        root,
        old_name: oldName,
        new_name: newName,
      }).unwrap();
      closeDialog();
      setFeedback({
        tone: "success",
        text: `Renamed ${oldName} to ${response.name}`,
      });
    } catch (error) {
      setDialogError(gitErrorText(error));
    }
  };

  const remove = async (name: string, force: boolean) => {
    setDialogError(null);
    try {
      await deleteBranch({
        root,
        name,
        ...(force ? { force: true } : {}),
      }).unwrap();
      closeDialog();
      setFeedback({ tone: "success", text: `Deleted branch ${name}` });
    } catch (error) {
      if (gitErrorCode(error) === "not_merged") {
        openDialog({ kind: "delete", name, notMerged: true });
        return;
      }
      setDialogError(gitErrorText(error));
    }
  };

  const checkoutRevision = async (revision: string, isTag: boolean) => {
    setDialogError(null);
    try {
      await checkoutRef({
        root,
        name: revision,
        kind: isTag ? "tag" : "revision",
      }).unwrap();
      closeDialog();
      setFeedback({ tone: "success", text: `Checked out ${revision}` });
    } catch (error) {
      setDialogError(gitErrorText(error));
    }
  };

  const branchActions = useCallback(
    (ref: BranchRef): GitMenuAction[] => {
      const checkoutAction: GitMenuAction = {
        id: "checkout",
        label: "Checkout",
        onSelect: () => checkoutBranchRef(ref),
        disabled: ref.current || blockedReason !== null,
        hint: blockedReason ?? undefined,
      };
      if (ref.kind === "tag") return [checkoutAction];
      const actions: GitMenuAction[] = [
        checkoutAction,
        {
          id: "new-branch",
          label: `New Branch from '${ref.label}'…`,
          onSelect: () => openNewBranchFromRef(ref),
        },
      ];
      if (ref.kind === "local") {
        actions.push({
          id: "rename",
          label: "Rename…",
          onSelect: () => openDialog({ kind: "rename", name: ref.name }),
          separatorBefore: true,
        });
        if (!ref.current) {
          actions.push({
            id: "delete",
            label: "Delete…",
            danger: true,
            onSelect: () =>
              openDialog({ kind: "delete", name: ref.name, notMerged: false }),
          });
        }
      }
      return actions;
    },
    [blockedReason, checkoutBranchRef, openDialog, openNewBranchFromRef],
  );

  const commitActions = useCallback(
    (commit: GitCommitLogEntry): GitMenuAction[] => [
      {
        id: "copy",
        label: "Copy Revision Number",
        onSelect: () => copy(commit.oid),
      },
      {
        id: "checkout",
        label: "Checkout Revision",
        onSelect: () => void checkout(commit.oid, "revision"),
        disabled: blockedReason !== null,
        hint: blockedReason ?? undefined,
        separatorBefore: true,
      },
      {
        id: "new-branch",
        label: "New Branch…",
        onSelect: () =>
          openNewBranch({ startPoint: commit.oid, label: commit.short_oid }),
      },
    ],
    [blockedReason, checkout, copy, openNewBranch],
  );

  let dialogs: ReactNode = null;
  if (dialog?.kind === "new") {
    dialogs = (
      <NewBranchDialog
        busy={createState.isLoading}
        defaultName={dialog.defaultName}
        error={dialogError}
        existing={localNames}
        onCancel={closeDialog}
        onConfirm={(name, checkoutAfter) =>
          void create(name, checkoutAfter, dialog.startPoint)
        }
        startLabel={dialog.startLabel}
      />
    );
  } else if (dialog?.kind === "rename") {
    dialogs = (
      <RenameBranchDialog
        busy={renameState.isLoading}
        error={dialogError}
        existing={localNames}
        name={dialog.name}
        onCancel={closeDialog}
        onConfirm={(newName) => void rename(dialog.name, newName)}
      />
    );
  } else if (dialog?.kind === "delete") {
    dialogs = (
      <DeleteBranchDialog
        busy={deleteState.isLoading}
        error={dialogError}
        key={dialog.notMerged ? "force" : "safe"}
        name={dialog.name}
        notMerged={dialog.notMerged}
        onCancel={closeDialog}
        onConfirm={(force) => void remove(dialog.name, force)}
      />
    );
  } else if (dialog?.kind === "revision") {
    dialogs = (
      <CheckoutRevisionDialog
        busy={checkoutState.isLoading}
        error={dialogError}
        onCancel={closeDialog}
        onConfirm={(revision, isTag) => void checkoutRevision(revision, isTag)}
        tags={tags}
      />
    );
  } else if (dialog?.kind === "push" && currentBranch) {
    dialogs = (
      <PushDialog
        ahead={status?.ahead ?? null}
        branch={currentBranch}
        busy={pushState.isLoading}
        error={dialogError}
        key={dialog.setUpstream ? "upstream" : "tracked"}
        onCancel={closeDialog}
        onConfirm={(target) => void push(target)}
        remotes={remotes}
        root={root}
        setUpstreamMode={dialog.setUpstream}
        upstreamMismatch={dialog.mismatch === true}
        upstream={status?.upstream ?? null}
      />
    );
  }

  return {
    root,
    status,
    branches,
    remotes,
    currentBranch,
    blockedReason,
    updateBlockedReason,
    canPush: Boolean(currentBranch) && !status?.headDetached,
    fetching: fetchState.isLoading,
    pulling: pullState.isLoading,
    pushing: pushState.isLoading,
    checkingOut: checkoutState.isLoading,
    feedback,
    clearFeedback: () => setFeedback(null),
    fetch,
    pull,
    openPush,
    openNewBranch,
    openCheckoutRevision: () => openDialog({ kind: "revision" }),
    checkoutBranchRef,
    branchActions,
    commitActions,
    dialogs,
  };
}

export type GitActionsController = ReturnType<typeof useGitActions>;

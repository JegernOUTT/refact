import { useEffect, useRef, useState } from "react";
import { Badge, Button, Dialog, Spinner } from "../../../components/ui";
import {
  useDeleteWorktreeMutation,
  useGetWorktreesSummaryQuery,
  useListWorktreesQuery,
  useOpenWorktreeMutation,
  type WorktreeRecordView,
  type WorktreeMeta,
} from "../../../services/refact/worktrees";
import { useEventsBusForIDE } from "../../../hooks/useEventBusForIDE";
import { useCopyToClipboard } from "../../../hooks/useCopyToClipboard";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import {
  selectRuntimeById,
  selectThreadWorktreeById,
  setThreadWorktree,
  useThreadId,
} from "../../Chat/Thread";
import { WorktreeCard } from "../../Worktrees/WorktreeCard";
import {
  sameWorktree,
  selectedFirst,
  worktreeInspection,
} from "../../Worktrees/WorktreeCard.helpers";
import { openWorktreeDiff } from "../../Worktrees/worktreeDiffSlice";
import { MergeWorktreeModal } from "../../Worktrees/MergeWorktreeModal";
import { worktreeErrorText } from "../../Worktrees/worktreeError";
import { selectConfig, selectHost } from "../../Config/configSlice";
import styles from "./GitPanel.module.css";
import cardStyles from "../../Worktrees/Worktrees.module.css";

export function WorktreesSection({ workspaceRoot }: { workspaceRoot: string }) {
  const config = useAppSelector(selectConfig);
  const chatId = useThreadId();
  const project = useAppSelector((state) => state.current_project);
  return (
    <WorktreesSectionInstance
      key={JSON.stringify([
        workspaceRoot,
        chatId,
        config.lspUrl,
        config.lspPort,
        config.apiKey,
        project,
      ])}
      workspaceRoot={workspaceRoot}
    />
  );
}

function WorktreesSectionInstance({
  workspaceRoot,
}: {
  workspaceRoot: string;
}) {
  const dispatch = useAppDispatch();
  const chatId = useThreadId();
  const attached = useAppSelector((state) =>
    selectThreadWorktreeById(state, chatId),
  );
  const busy = useAppSelector((state) => {
    const runtime = selectRuntimeById(state, chatId);
    return (
      runtime?.streaming === true ||
      runtime?.waiting_for_response === true ||
      runtime?.session_state === "generating" ||
      runtime?.session_state === "executing_tools" ||
      runtime?.thread.task_meta?.role === "agent" ||
      runtime?.thread.task_meta?.role === "agents"
    );
  });
  const worktrees = useListWorktreesQuery({
    source_workspace_root: workspaceRoot,
  });
  const inventory = useGetWorktreesSummaryQuery({
    source_workspace_root: workspaceRoot,
  });
  const [openWorktree] = useOpenWorktreeMutation();
  const [deleteWorktree, deleteState] = useDeleteWorktreeMutation();
  const copyToClipboard = useCopyToClipboard();
  const host = useAppSelector(selectHost);
  const hostCanOpenFolder = ["vscode", "jetbrains", "ide"].includes(host);
  const { openFolderInNewWindow } = useEventsBusForIDE();
  const [target, setTarget] = useState<{
    meta: WorktreeMeta;
    record?: WorktreeRecordView;
  } | null>(null);
  const [dialog, setDialog] = useState<"merge" | "delete" | null>(null);
  const [deleteBranch, setDeleteBranch] = useState(false);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(true);
  const latestAttachment = useRef(attached);
  latestAttachment.current = attached;
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const detachRemoved = (record: { meta: WorktreeMeta }) => {
    if (mounted.current && sameWorktree(record.meta, latestAttachment.current))
      dispatch(setThreadWorktree({ chatId, worktree: null }));
  };
  const handleOpen = async (record: { meta: WorktreeMeta }) => {
    try {
      const response = await openWorktree({
        id: record.meta.id,
        source_workspace_root: record.meta.source_workspace_root,
      }).unwrap();
      if (!mounted.current) return;
      if (response.can_open_folder && hostCanOpenFolder) {
        openFolderInNewWindow(response.path);
        setFeedback("Opening worktree in a new window.");
      } else {
        copyToClipboard(response.path);
        setFeedback(`Copied ${response.path}`);
      }
    } catch (openError) {
      if (mounted.current) setError(worktreeErrorText(openError));
    }
  };
  const handleDelete = async () => {
    if (
      !target ||
      busy ||
      target.meta.kind === "task_agent" ||
      deleteState.isLoading
    )
      return;
    const record = target;
    setError(null);
    try {
      const response = await deleteWorktree({
        id: record.meta.id,
        source_workspace_root: record.meta.source_workspace_root,
        delete_branch: deleteBranch,
        force_referenced: true,
      }).unwrap();
      if (!mounted.current) return;
      if (!response.deleted) {
        setError(
          "Delete not confirmed; worktree retained. Inspect it before retrying.",
        );
        return;
      }
      detachRemoved(record);
      setDialog(null);
      setFeedback(
        `Deleted ${record.meta.branch ?? record.meta.id}${
          response.warnings.length ? `. ${response.warnings.join(" ")}` : ""
        }`,
      );
    } catch (deleteError) {
      if (mounted.current) setError(worktreeErrorText(deleteError));
    }
  };
  const records = selectedFirst(worktrees.data?.worktrees ?? [], attached);
  const fallback =
    attached?.source_workspace_root === workspaceRoot &&
    !records.some((record) => sameWorktree(record.meta, attached));
  return (
    <section className={styles.section} aria-labelledby="git-worktrees-heading">
      <header className={styles.sectionHeader}>
        <div>
          <h2 id="git-worktrees-heading">Worktrees</h2>
          <p>Inspect and manage checkouts without changing chat scope.</p>
        </div>
        <Badge tone="muted">{records.length}</Badge>
      </header>
      {feedback && (
        <p className={styles.successText} role="status">
          {feedback}
        </p>
      )}
      {error && (
        <p className={styles.errorText} role="alert">
          {error}
        </p>
      )}
      {worktrees.isLoading && <Spinner label="Loading worktrees" size="sm" />}
      {worktrees.error && (
        <p className={styles.errorText} role="alert">
          {worktreeErrorText(worktrees.error)}
        </p>
      )}
      {!worktrees.isLoading && !worktrees.error && records.length === 0 && (
        <p className={styles.emptyText}>No registered worktrees.</p>
      )}
      <div className={cardStyles.recordList}>
        {!attached && (
          <WorktreeCard
            openCopiesPath
            selected
            mainPath={workspaceRoot}
            mainBranch={worktrees.data?.source_current_branch}
            onCopy={() => copyToClipboard(workspaceRoot)}
            onOpen={() => copyToClipboard(workspaceRoot)}
          />
        )}
        {fallback && (
          <WorktreeCard
            openCopiesPath
            selected
            worktree={attached}
            unavailable
            inspection={worktreeInspection(inventory.data, attached)}
            onDiff={() =>
              dispatch(
                openWorktreeDiff({
                  worktreeId: attached.id,
                  sourceWorkspaceRoot: attached.source_workspace_root,
                  root: attached.root,
                  label: attached.branch ?? undefined,
                }),
              )
            }
            onCopy={() => copyToClipboard(attached.root)}
            onOpen={() => copyToClipboard(attached.root)}
          />
        )}
        {records.map((record) => (
          <WorktreeCard
            openCopiesPath={!hostCanOpenFolder}
            key={record.meta.id}
            worktree={record.meta}
            record={record}
            inspection={worktreeInspection(inventory.data, record.meta)}
            selected={sameWorktree(record.meta, attached)}
            managementDisabled={Boolean(busy)}
            onDiff={() =>
              dispatch(
                openWorktreeDiff({
                  worktreeId: record.meta.id,
                  sourceWorkspaceRoot: record.meta.source_workspace_root,
                  root: record.meta.root,
                  label: record.meta.branch ?? undefined,
                }),
              )
            }
            onMerge={() => {
              setTarget({ meta: record.meta, record });
              setDialog("merge");
            }}
            onDelete={() => {
              setTarget({ meta: record.meta, record });
              setDeleteBranch(false);
              setError(null);
              setDialog("delete");
            }}
            onCopy={() => copyToClipboard(record.meta.root)}
            onOpen={() => void handleOpen(record)}
          />
        ))}
        {attached && (
          <WorktreeCard
            openCopiesPath
            selected={false}
            mainPath={workspaceRoot}
            mainBranch={worktrees.data?.source_current_branch}
            onCopy={() => copyToClipboard(workspaceRoot)}
            onOpen={() => copyToClipboard(workspaceRoot)}
          />
        )}
      </div>
      <MergeWorktreeModal
        open={dialog === "merge" && !busy}
        worktree={target?.meta}
        record={target?.record}
        onOpenChange={(open) => {
          if (!open) setDialog(null);
        }}
        onMerged={(response) => {
          if (
            target &&
            (response.cleanup?.worktree_deleted === true ||
              response.cleanup?.registry_deleted === true)
          )
            detachRemoved(target);
        }}
        onOpenWorktree={target ? () => handleOpen(target) : undefined}
      />
      <Dialog
        open={dialog === "delete" && !busy}
        onOpenChange={(open) => {
          if (!open && !deleteState.isLoading) setDialog(null);
        }}
      >
        <Dialog.Content maxWidth="420px">
          <Dialog.Title>Delete worktree</Dialog.Title>
          <Dialog.Description>
            Delete or discard {target?.meta.branch ?? target?.meta.id} from
            disk.
          </Dialog.Description>
          <div className={cardStyles.modalFields}>
            <span className={cardStyles.path}>{target?.meta.root}</span>
            {(target?.record?.reference_count ??
              target?.meta.reference_count ??
              0) > 0 && (
              <p className={cardStyles.warningBox}>
                {(target?.record?.reference_count ??
                  target?.meta.reference_count) === 1
                  ? "This worktree has 1 reference."
                  : `This worktree is shared by ${
                      target?.record?.reference_count ??
                      target?.meta.reference_count
                    } references.`}{" "}
                Deleting it may affect chats that use this worktree.
              </p>
            )}
            <label className={cardStyles.checkboxRow}>
              <input
                type="checkbox"
                checked={deleteBranch}
                onChange={(event) =>
                  setDeleteBranch(event.currentTarget.checked)
                }
                disabled={deleteState.isLoading}
              />
              Delete git branch too
            </label>
            {error && (
              <p role="alert" className={cardStyles.errorBox}>
                {error}
              </p>
            )}
          </div>
          <div className={cardStyles.modalActions}>
            <Dialog.Close asChild>
              <Button variant="soft" disabled={deleteState.isLoading}>
                Cancel
              </Button>
            </Dialog.Close>
            <Button
              variant="danger"
              loading={deleteState.isLoading}
              onClick={() => void handleDelete()}
            >
              Delete worktree
            </Button>
          </div>
        </Dialog.Content>
      </Dialog>
    </section>
  );
}

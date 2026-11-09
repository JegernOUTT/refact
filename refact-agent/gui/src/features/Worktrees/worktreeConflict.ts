import type {
  MergeWorktreeResponse,
  WorktreeMeta,
  WorktreeRecordView,
  WorktreeSyncResponse,
} from "../../services/refact";

type ConflictPromptArgs = {
  worktree?: WorktreeMeta | null;
  record?: WorktreeRecordView | null;
  response?: MergeWorktreeResponse | null;
  sync?: WorktreeSyncResponse | null;
  files: string[];
  taskId?: string;
  cardId?: string;
};

export function mergeConflictFiles(response: MergeWorktreeResponse): string[] {
  return response.conflict?.files ?? response.conflict_files ?? [];
}

function syncGuidance(sync: WorktreeSyncResponse | null | undefined): string {
  const operation = sync?.operation;
  const state = operation
    ? `Native synchronization receipt: operation_id=${operation.operation_id} expected_revision=${operation.revision} (phase ${operation.phase}, status ${sync.status}). Pinned source ${operation.source_head} against ${operation.target_branch} ${operation.target_head}.`
    : "No native synchronization operation is active.";
  return [
    state,
    "Use normal shell Git in the authorized source worktree: inspect git status and the current diff, merge the target if integration is needed, resolve the conflicted files, git add only the intended resolved paths, and git commit to complete the merge.",
    "Preserve unrelated work, inspect any existing merge before starting another, and run verification on the integrated candidate. The optional synchronization panel remains available for user-managed native operations.",
  ].join(" ");
}

export function buildWorktreeConflictPrompt({
  worktree,
  record,
  response,
  sync,
  files,
  taskId,
  cardId,
}: ConflictPromptArgs): string {
  const meta = record?.meta ?? worktree;
  const branch =
    response?.source_branch ?? meta?.branch ?? "the worktree branch";
  const target =
    response?.target_branch ??
    sync?.operation?.target_branch ??
    meta?.base_branch ??
    "the target branch";
  const root = meta?.root ?? "the active worktree";
  const syncFiles =
    sync?.operation?.target_branch === target
      ? sync.conflicts.map((conflict) => conflict.path)
      : [];
  const allFiles = files.length > 0 ? files : syncFiles;
  const fileLines =
    allFiles.length > 0
      ? allFiles.map((file) => `- ${file}`).join("\n")
      : "- No conflicted files were reported.";
  const taskLine = taskId
    ? `Task: ${taskId}${cardId ? ` / ${cardId}` : ""}\n`
    : "";
  return `${taskLine}Synchronize worktree ${root} with its target branch.

Branch: ${branch}
Target: ${target}

Conflicted files:
${fileLines}

${syncGuidance(sync)}`;
}

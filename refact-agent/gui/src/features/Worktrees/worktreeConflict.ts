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
  toolAvailable?: boolean;
  files: string[];
  taskId?: string;
  cardId?: string;
  selector?: { worktree_id: string; card_id?: string; child_chat_id?: string };
};

export function mergeConflictFiles(response: MergeWorktreeResponse): string[] {
  return response.conflict?.files ?? response.conflict_files ?? [];
}

function syncGuidance(
  sync: WorktreeSyncResponse | null | undefined,
  toolAvailable: boolean | undefined,
  selector?: ConflictPromptArgs["selector"],
): string {
  const operation = sync?.operation;
  if (!sync || !operation) {
    return [
      "No native synchronization operation is active, so there is nothing to resolve from chat yet.",
      "Do not merge manually. Ask the user to press Synchronize with target in the worktree panel.",
    ].join(" ");
  }
  const identity = `operation_id=${operation.operation_id} expected_revision=${operation.revision} (phase ${operation.phase}, status ${sync.status})`;
  const pins = `Pinned source ${operation.source_head} against ${operation.target_branch} ${operation.target_head}.`;
  if (toolAvailable === false) {
    return [
      `A native synchronization is active: ${identity}. ${pins}`,
      "The worktree_sync tool is not available in this chat mode, so do not call it and do not merge manually.",
      "Explain the conflicts to the user and ask them to resolve, continue or abort with the synchronization panel controls.",
    ].join(" ");
  }
  return [
    `A native synchronization is active: ${identity}. ${pins}`,
    toolAvailable === undefined
      ? "Tool availability in this chat mode is unknown. If worktree_sync is unavailable, use the native synchronization panel; never merge manually."
      : "Use the worktree_sync tool, never a manual merge.",
    `If available, call ${JSON.stringify({
      action: "status",
      ...selector,
      target_branch: operation.target_branch,
    })} first to re-read the current operation_id and revision. The server validates authorization.`,
    "Fix each conflicted file with normal edit tools, then call resolve with a choice per path (working_tree, source, target or delete) and the current operation_id and expected_revision.",
    "When no conflicts remain call continue with the latest revision for this user-requested operation; do not abort or start unrelated operations.",
  ].join(" ");
}

export function buildWorktreeConflictPrompt({
  worktree,
  record,
  response,
  sync,
  toolAvailable,
  files,
  taskId,
  cardId,
  selector,
}: ConflictPromptArgs): string {
  const meta = record?.meta ?? worktree;
  const branch =
    response?.source_branch ?? meta?.branch ?? "the worktree branch";
  const target =
    sync?.operation?.target_branch ??
    response?.target_branch ??
    meta?.base_branch ??
    "the target branch";
  const root = meta?.root ?? "the active worktree";
  const syncFiles = sync?.conflicts.map((conflict) => conflict.path) ?? [];
  const allFiles = syncFiles.length > 0 ? syncFiles : files;
  const fileLines =
    allFiles.length > 0
      ? allFiles.map((file) => `- ${file}`).join("\n")
      : "- No conflicted files were reported.";
  const taskLine = taskId
    ? `Task: ${taskId}${cardId ? ` / ${cardId}` : ""}\n`
    : "";
  const instructions = sync?.instructions ?? response?.conflict?.instructions;
  return `${taskLine}Synchronize worktree ${root} with its target branch.

Branch: ${branch}
Target: ${target}

Conflicted files:
${fileLines}

${syncGuidance(sync, toolAvailable, selector)}${
    instructions ? `\n\nBackend instructions:\n${instructions}` : ""
  }`;
}

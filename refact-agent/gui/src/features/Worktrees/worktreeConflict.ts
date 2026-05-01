import type {
  MergeWorktreeResponse,
  WorktreeMeta,
  WorktreeRecordView,
} from "../../services/refact";

type ConflictPromptArgs = {
  worktree?: WorktreeMeta | null;
  record?: WorktreeRecordView | null;
  response?: MergeWorktreeResponse | null;
  files: string[];
  taskId?: string;
  cardId?: string;
};

export function mergeConflictFiles(response: MergeWorktreeResponse): string[] {
  return response.conflict?.files ?? response.conflict_files ?? [];
}

export function buildWorktreeConflictPrompt({
  worktree,
  record,
  response,
  files,
  taskId,
  cardId,
}: ConflictPromptArgs): string {
  const meta = record?.meta ?? worktree;
  const branch =
    response?.source_branch ?? meta?.branch ?? "the worktree branch";
  const target =
    response?.target_branch ?? meta?.base_branch ?? "the target branch";
  const root = meta?.root ?? "the active worktree";
  const fileLines =
    files.length > 0
      ? files.map((file) => `- ${file}`).join("\n")
      : "- No conflicted files were reported.";
  const taskLine = taskId
    ? `Task: ${taskId}${cardId ? ` / ${cardId}` : ""}\n`
    : "";
  const instructions = response?.conflict?.instructions;
  const mergeWasStopped = response?.conflict?.aborted === true;
  const resolutionInstructions = mergeWasStopped
    ? [
        "The merge was stopped and cleaned up after conflict detection, so conflict markers will not be present in these files.",
        "Please merge it yourself by applying the intended source branch changes against the target branch, then verify the result.",
      ].join(" ")
    : "Please inspect the conflict markers, preserve the intended changes, update the files, and verify the result.";
  return `${taskLine}Resolve the git merge conflicts in worktree ${root}.

Branch: ${branch}
Target: ${target}

Conflicted files:
${fileLines}

${resolutionInstructions}${
    instructions ? `\n\nBackend instructions:\n${instructions}` : ""
  }`;
}

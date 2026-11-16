import type { RootState } from "../../../app/store";

const EMPTY: readonly string[] = [];
const single = new Map<string, readonly string[]>();

/**
 * Every root a chat's paths may be relative to: its worktree when it has one,
 * otherwise all configured workspace roots. Returns stable arrays.
 */
export function selectChatWorkspaceRoots(
  state: RootState,
  chatId: string,
): readonly string[] {
  const worktree = state.chat.threads[chatId]?.thread.worktree?.root;
  if (worktree) {
    let cached = single.get(worktree);
    if (!cached) {
      cached = [worktree];
      single.set(worktree, cached);
    }
    return cached;
  }
  return state.current_project.workspaceRoots ?? EMPTY;
}

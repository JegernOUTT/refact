import { worktreeErrorText } from "../../Worktrees/worktreeError";

export type GitErrorView = {
  code: string | null;
  message: string;
  output: string | null;
  files: string[];
};

const FRIENDLY_MESSAGES: Record<string, string> = {
  bad_request: "The request was rejected by the engine.",
  not_found: "The repository or file could not be found.",
  git_error: "Git reported an error.",
  conflict: "Resolve the merge conflicts before continuing.",
  nothing_to_commit: "There is nothing to commit.",
  git_unavailable: "Git is not available for this workspace.",
  timeout: "The Git operation timed out.",
};

const RAW_OUTPUT_MESSAGES: Record<string, string> = {
  hook_failed: "A Git hook rejected the commit.",
  checkout_conflict:
    "Checkout would overwrite local changes. Commit or roll back these files first.",
  already_exists: "A branch with this name already exists.",
  current_branch:
    "The current branch can't be deleted. Check out another one first.",
  not_merged: "The branch is not fully merged.",
  no_upstream: "The current branch has no upstream branch.",
  not_fast_forward:
    "Local and remote branches have diverged — merge or rebase outside the app.",
  rejected: "Push was rejected by the remote. Update the project first.",
  auth_failed:
    "Authentication failed — configure a credential helper or SSH agent for this remote.",
  remote_error: "The remote reported an error.",
};

function errorData(error: unknown): unknown {
  if (typeof error === "object" && error !== null && "data" in error) {
    return error.data;
  }
  return error;
}

function parseData(data: unknown): unknown {
  if (typeof data !== "string") return data;
  try {
    return JSON.parse(data) as unknown;
  } catch {
    return data;
  }
}

export function conflictFiles(detail: string): string[] {
  return detail
    .split(/\r?\n/u)
    .filter((line) => /^\s+\S/u.test(line))
    .map((line) => line.trim());
}

export function gitErrorCode(error: unknown): string | null {
  const data = parseData(errorData(error));
  if (
    typeof data === "object" &&
    data !== null &&
    "code" in data &&
    typeof data.code === "string"
  ) {
    return data.code;
  }
  return null;
}

export function gitErrorText(error: unknown): GitErrorView {
  const data = parseData(errorData(error));
  if (
    typeof data === "object" &&
    data !== null &&
    "code" in data &&
    typeof data.code === "string"
  ) {
    const detail =
      "error" in data && typeof data.error === "string" ? data.error : "";
    const rawOutputMessage = RAW_OUTPUT_MESSAGES[data.code] as
      | string
      | undefined;
    if (rawOutputMessage) {
      return {
        code: data.code,
        message: rawOutputMessage,
        output: detail || null,
        files: data.code === "checkout_conflict" ? conflictFiles(detail) : [],
      };
    }
    const friendly = FRIENDLY_MESSAGES[data.code] as string | undefined;
    return {
      code: data.code,
      message:
        friendly && detail && detail !== friendly
          ? `${friendly} ${detail}`
          : friendly ?? detail,
      output: null,
      files: [],
    };
  }
  return {
    code: null,
    message: worktreeErrorText(error),
    output: null,
    files: [],
  };
}

import { worktreeErrorText } from "../../Worktrees/worktreeError";

export type GitErrorView = {
  code: string | null;
  message: string;
  output: string | null;
};

const FRIENDLY_MESSAGES: Record<string, string> = {
  bad_request: "The request was rejected by the engine.",
  not_found: "The repository or file could not be found.",
  git_error: "Git reported an error.",
  conflict: "Resolve the merge conflicts before continuing.",
  nothing_to_commit: "There is nothing to commit.",
  hook_failed: "A Git hook rejected the commit.",
  git_unavailable: "Git is not available for this workspace.",
  timeout: "The Git operation timed out.",
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
    const friendly = FRIENDLY_MESSAGES[data.code] as string | undefined;
    if (data.code === "hook_failed") {
      return {
        code: data.code,
        message: friendly ?? detail,
        output: detail || null,
      };
    }
    return {
      code: data.code,
      message:
        friendly && detail && detail !== friendly
          ? `${friendly} ${detail}`
          : friendly ?? detail,
      output: null,
    };
  }
  return { code: null, message: worktreeErrorText(error), output: null };
}

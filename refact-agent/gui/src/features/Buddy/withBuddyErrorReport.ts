import { reportBuddyFrontendError } from "./reportBuddyFrontendError";

export function withBuddyErrorReport<T>(
  fn: () => T,
  args: {
    source: "react_root_render" | "react_recoverable";
    sourceFile: string;
    toolName: string;
  },
): T {
  try {
    return fn();
  } catch (error) {
    void reportBuddyFrontendError({
      source: args.source,
      error,
      sourceFile: args.sourceFile,
      toolName: args.toolName,
    });
    throw error;
  }
}

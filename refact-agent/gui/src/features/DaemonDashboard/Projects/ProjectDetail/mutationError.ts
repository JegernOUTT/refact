export function mutationError(error: unknown): string | null {
  if (!error || typeof error !== "object") return null;
  if ("data" in error) {
    const data = error.data;
    if (typeof data === "string") return data;
    if (data && typeof data === "object") {
      if ("error" in data && typeof data.error === "string") return data.error;
      if ("detail" in data && typeof data.detail === "string")
        return data.detail;
    }
  }
  if ("message" in error && typeof error.message === "string") {
    return error.message;
  }
  return "Project action failed.";
}

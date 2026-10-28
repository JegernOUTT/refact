export type LogFilterRequest = { path?: string; ref?: string };

export type BranchFilter =
  | { kind: "all" }
  | { kind: "head" }
  | { kind: "ref"; name: string };

export type LogFilters = {
  branch: BranchFilter;
  text: string;
  author: string;
  path: string;
};

export const DEFAULT_LOG_FILTERS: LogFilters = {
  branch: { kind: "all" },
  text: "",
  author: "",
  path: "",
};

export function branchFilterLabel(branch: BranchFilter): string {
  if (branch.kind === "all") return "All";
  if (branch.kind === "head") return "HEAD";
  return branch.name;
}

export function hasContentFilter(filters: LogFilters): boolean {
  return Boolean(filters.text || filters.author || filters.path);
}

/** Filters for a one-shot Show History / Show Log request. */
export function filtersForRequest(request: LogFilterRequest): LogFilters {
  return {
    ...DEFAULT_LOG_FILTERS,
    path: request.path ?? "",
    branch: request.ref ? { kind: "ref", name: request.ref } : { kind: "all" },
  };
}

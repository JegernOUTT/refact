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

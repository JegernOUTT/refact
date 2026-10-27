import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  useGetGitLogQuery,
  useLazyGetGitLogQuery,
  type GitCommitLogEntry,
  type GitLogRequest,
} from "../../../../services/refact/gitRead";
import {
  extendCommitGraph,
  layoutCommitGraph,
  type GraphLayout,
} from "./commitGraphLayout";
import type { BranchFilter, LogFilters } from "./logFilters";

export const LOG_PAGE_SIZE = 200;

function branchParams(
  branch: BranchFilter,
): Pick<GitLogRequest, "scope" | "ref"> {
  if (branch.kind === "ref") return { ref: branch.name };
  return { scope: branch.kind };
}

export function logRequest(
  root: string,
  filters: LogFilters,
  skip: number,
): GitLogRequest {
  return {
    root,
    limit: LOG_PAGE_SIZE,
    skip,
    ...branchParams(filters.branch),
    text: filters.text || undefined,
    author: filters.author || undefined,
    path: filters.path || undefined,
  };
}

function appendUnique(
  current: GitCommitLogEntry[],
  incoming: GitCommitLogEntry[],
): GitCommitLogEntry[] {
  const known = new Set(current.map((commit) => commit.oid));
  return [...current, ...incoming.filter((commit) => !known.has(commit.oid))];
}

type ExtraPages = {
  commits: GitCommitLogEntry[];
  lastPageSize: number;
};

function useIncrementalGraph(
  commits: GitCommitLogEntry[],
  enabled: boolean,
): GraphLayout | null {
  const cache = useRef<{ commits: GitCommitLogEntry[]; layout: GraphLayout }>();
  return useMemo(() => {
    if (!enabled) return null;
    const previous = cache.current;
    const inputs = (list: GitCommitLogEntry[]) =>
      list.map((commit) => ({
        oid: commit.oid,
        parents: commit.parents ?? [],
      }));
    const extendsPrevious =
      previous !== undefined &&
      previous.commits.length <= commits.length &&
      previous.commits.every((commit, index) => commits[index] === commit);
    const layout = extendsPrevious
      ? extendCommitGraph(
          previous.layout,
          inputs(commits.slice(previous.commits.length)),
        )
      : layoutCommitGraph(inputs(commits));
    cache.current = { commits, layout };
    return layout;
  }, [commits, enabled]);
}

export function useCommitLog(root: string, filters: LogFilters) {
  const firstRequest = useMemo(
    () => logRequest(root, filters, 0),
    [root, filters],
  );
  const head = useGetGitLogQuery(firstRequest);
  const [fetchPage, pageState] = useLazyGetGitLogQuery();
  const [extra, setExtra] = useState<ExtraPages | null>(null);
  const headRoot = head.data?.roots[0];
  const { refetch } = head;

  useEffect(() => {
    setExtra(null);
  }, [headRoot]);

  const commits = useMemo(
    () => appendUnique(headRoot?.commits ?? [], extra?.commits ?? []),
    [extra, headRoot],
  );
  const lastPageSize = extra
    ? extra.lastPageSize
    : headRoot?.commits.length ?? 0;
  const hasMore =
    !head.isFetching &&
    headRoot !== undefined &&
    lastPageSize === LOG_PAGE_SIZE;

  const loadingMore = useRef(false);
  const loadMore = useCallback(async () => {
    if (!hasMore || loadingMore.current) return;
    loadingMore.current = true;
    try {
      const response = await fetchPage(
        logRequest(root, filters, commits.length),
      ).unwrap();
      const page = response.roots[0]?.commits ?? [];
      setExtra((current) => ({
        commits: appendUnique(current?.commits ?? [], page),
        lastPageSize: page.length,
      }));
    } catch {
      // Surfaced through pageState.error; the next scroll or click retries.
    } finally {
      loadingMore.current = false;
    }
  }, [commits.length, fetchPage, filters, hasMore, root]);

  const graphEnabled =
    !filters.text &&
    !filters.author &&
    !filters.path &&
    commits.length > 0 &&
    commits.every((commit) => Array.isArray(commit.parents));
  const graph = useIncrementalGraph(commits, graphEnabled);

  return {
    commits,
    graph,
    headOid: headRoot?.head_oid ?? null,
    scanTruncated: headRoot?.scan_truncated ?? false,
    isLoading: head.isLoading || (head.isFetching && commits.length === 0),
    isFetching: head.isFetching,
    loadingMore: pageState.isFetching,
    error: head.error,
    pageError: pageState.error,
    hasMore,
    loadMore,
    refetch,
  };
}

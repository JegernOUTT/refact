import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties } from "react";

import { useAppDispatch, useAppSelector } from "../../../../hooks";
import { useGetGitBranchesQuery } from "../../../../services/refact/gitRead";
import { gitErrorText } from "../gitErrors";
import { selectGitLogLayout, setGitLogLayout } from "../gitPanelSlice";
import { BranchesPane, type BranchActionsRenderer } from "./BranchesPane";
import { allBranchRefs, buildBranchTree } from "./branchesTreeModel";
import { CommitDetailsPane } from "./CommitDetailsPane";
import { CommitsPane } from "./CommitsPane";
import { DEFAULT_LOG_FILTERS, type LogFilters } from "./logFilters";
import { PaneSplitter } from "./PaneSplitter";
import { useCommitLog } from "./useCommitLog";
import styles from "./GitLog.module.css";

const BRANCHES_MIN = 160;
const BRANCHES_MAX = 480;
const DETAILS_MIN = 260;
const DETAILS_MAX = 720;

export type GitLogProps = {
  root: string;
  /** Extension point for branch actions (checkout, rename, …). */
  renderBranchActions?: BranchActionsRenderer;
};

function useRefetchOnFocus(refetch: () => void) {
  const ref = useRef(refetch);
  ref.current = refetch;
  useEffect(() => {
    const onFocus = () => ref.current();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);
}

export function GitLog({ root, renderBranchActions }: GitLogProps) {
  const dispatch = useAppDispatch();
  const layout = useAppSelector(selectGitLogLayout);
  const containerRef = useRef<HTMLDivElement>(null);
  const [filters, setFilters] = useState<LogFilters>(DEFAULT_LOG_FILTERS);
  const [selectedOid, setSelectedOid] = useState<string | null>(null);
  const branches = useGetGitBranchesQuery(root);
  const log = useCommitLog(root, filters);
  const branchData = branches.data?.roots[0];
  const refs = useMemo(
    () => (branchData ? allBranchRefs(buildBranchTree(branchData)) : []),
    [branchData],
  );
  const loaded = useMemo(
    () => new Set(log.commits.map((commit) => commit.oid)),
    [log.commits],
  );
  const { refetch: refetchLog } = log;
  const { refetch: refetchBranches } = branches;
  const refresh = useCallback(() => {
    void refetchLog();
    void refetchBranches();
  }, [refetchBranches, refetchLog]);
  useRefetchOnFocus(refresh);

  useEffect(() => {
    if (log.commits.length === 0) return;
    if (selectedOid && loaded.has(selectedOid)) return;
    setSelectedOid(log.commits[0].oid);
  }, [loaded, log.commits, selectedOid]);

  const updateFilters = useCallback((patch: Partial<LogFilters>) => {
    setFilters((current) => ({ ...current, ...patch }));
  }, []);

  const style = {
    "--git-log-branches-w": `${layout.branchesWidth}px`,
    "--git-log-details-w": `${layout.detailsWidth}px`,
  } as CSSProperties;

  return (
    <div className={styles.logLayout} ref={containerRef} style={style}>
      <section aria-label="Branches" className={styles.pane}>
        {branches.error ? (
          <p className={styles.errorText} role="alert">
            {gitErrorText(branches.error).message}
          </p>
        ) : (
          <BranchesPane
            data={branchData}
            filter={filters.branch}
            onNavigate={(branch) => updateFilters({ branch })}
            renderBranchActions={renderBranchActions}
          />
        )}
      </section>
      <PaneSplitter
        containerRef={containerRef}
        cssVar="--git-log-branches-w"
        edge="start"
        label="Resize branches pane"
        max={BRANCHES_MAX}
        min={BRANCHES_MIN}
        onCommit={(branchesWidth) =>
          dispatch(setGitLogLayout({ branchesWidth }))
        }
        value={layout.branchesWidth}
      />
      <section aria-label="Commit log" className={styles.pane} data-grow="true">
        <CommitsPane
          commits={log.commits}
          error={log.error}
          filters={filters}
          graph={log.graph}
          hasMore={log.hasMore}
          isFetching={log.isFetching}
          isLoading={log.isLoading}
          loadingMore={log.loadingMore}
          onFiltersChange={updateFilters}
          onLoadMore={() => void log.loadMore()}
          onRefresh={refresh}
          onSelect={setSelectedOid}
          pageError={log.pageError}
          refs={refs}
          scanTruncated={log.scanTruncated}
          selectedOid={selectedOid}
        />
      </section>
      <PaneSplitter
        containerRef={containerRef}
        cssVar="--git-log-details-w"
        edge="end"
        label="Resize details pane"
        max={DETAILS_MAX}
        min={DETAILS_MIN}
        onCommit={(detailsWidth) => dispatch(setGitLogLayout({ detailsWidth }))}
        value={layout.detailsWidth}
      />
      <section aria-label="Commit details" className={styles.pane}>
        <CommitDetailsPane
          isLoaded={(oid) => loaded.has(oid)}
          oid={selectedOid}
          onSelectCommit={setSelectedOid}
          root={root}
        />
      </section>
    </div>
  );
}

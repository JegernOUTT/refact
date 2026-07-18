import { useEffect, useState } from "react";

import { Badge, Button, Spinner } from "../../../components/ui";
import {
  useGetGitBranchesQuery,
  useGetGitLogQuery,
  type GitCommitLogEntry,
} from "../../../services/refact/gitRead";
import { worktreeErrorText } from "../../Worktrees/worktreeError";
import styles from "./GitPanel.module.css";

const PAGE_SIZE = 30;

function appendUnique(
  current: GitCommitLogEntry[],
  incoming: GitCommitLogEntry[],
): GitCommitLogEntry[] {
  const known = new Set(current.map((commit) => commit.oid));
  return [...current, ...incoming.filter((commit) => !known.has(commit.oid))];
}

export function BranchesLog({ root }: { root: string }) {
  const [skip, setSkip] = useState(0);
  const [commits, setCommits] = useState<GitCommitLogEntry[]>([]);
  const branches = useGetGitBranchesQuery(root);
  const log = useGetGitLogQuery({ root, limit: PAGE_SIZE, skip });
  const branchRoot = branches.data?.roots[0];
  const logRoot = log.data?.roots[0];

  useEffect(() => {
    setSkip(0);
    setCommits([]);
  }, [root]);

  useEffect(() => {
    if (!logRoot) return;
    setCommits((current) =>
      skip === 0 ? logRoot.commits : appendUnique(current, logRoot.commits),
    );
  }, [logRoot, skip]);

  return (
    <section className={styles.section} aria-labelledby="git-history-heading">
      <header className={styles.sectionHeader}>
        <div>
          <h2 id="git-history-heading">Branches &amp; history</h2>
          <p>Read-only branch information and recent commits.</p>
        </div>
        {branchRoot?.current ? (
          <Badge tone="accent">{branchRoot.current}</Badge>
        ) : null}
      </header>
      {branches.isLoading ? (
        <Spinner label="Loading branches" size="sm" />
      ) : branches.error ? (
        <p className={styles.errorText} role="alert">
          {worktreeErrorText(branches.error)}
        </p>
      ) : (
        <ul className={styles.branchList} aria-label="Branches">
          {(branchRoot?.branches ?? []).map((branch) => (
            <li key={branch.name}>
              <span className={styles.branchName}>{branch.name}</span>
              {branch.is_head ? <Badge tone="success">Current</Badge> : null}
              {branch.upstream ? (
                <span className={styles.mutedText}>{branch.upstream}</span>
              ) : null}
            </li>
          ))}
        </ul>
      )}
      <div className={styles.logHeader}>Recent commits</div>
      {log.isLoading && commits.length === 0 ? (
        <Spinner label="Loading commits" size="sm" />
      ) : log.error ? (
        <p className={styles.errorText} role="alert">
          {worktreeErrorText(log.error)}
        </p>
      ) : commits.length === 0 ? (
        <p className={styles.emptyText}>No commits found.</p>
      ) : (
        <ol className={styles.commitList}>
          {commits.map((commit) => (
            <li key={commit.oid}>
              <code>{commit.short_oid}</code>
              <span className={styles.commitMessage}>
                {commit.message_first_line}
              </span>
              <span className={styles.mutedText}>{commit.author_name}</span>
            </li>
          ))}
        </ol>
      )}
      {logRoot?.commits.length === PAGE_SIZE ? (
        <Button
          type="button"
          variant="soft"
          size="sm"
          loading={log.isFetching}
          disabled={log.isFetching}
          onClick={() => setSkip((value) => value + PAGE_SIZE)}
        >
          Load more
        </Button>
      ) : null}
    </section>
  );
}

import { Surface } from "../../../../../components/ui";
import type { DaemonWorker } from "../../../../../services/refact/daemon";
import type {
  GitBranchesRoot,
  GitCommitLogEntry,
  GitLogRoot,
  GitRootsResponse,
  GitStatusRoot,
} from "../../../../../services/refact/gitRead";
import { useProjectResource } from "../projectResource";
import { WorkerGate } from "../WorkerGate";
import { Fact, ResourceView } from "./shared";
import styles from "../ProjectDetail.module.css";

const GIT_LOG_LIMIT = 15;

type GitTabProps = {
  daemonBase: string;
  worker: DaemonWorker;
  onMutated: () => void;
};

function parseRoots<T>(data: unknown): GitRootsResponse<T> | null {
  if (!data || typeof data !== "object") return null;
  if (!("roots" in data) || !Array.isArray(data.roots)) return null;
  return data as GitRootsResponse<T>;
}

function parseStatus(data: unknown): GitRootsResponse<GitStatusRoot> | null {
  return parseRoots<GitStatusRoot>(data);
}

function parseLog(data: unknown): GitRootsResponse<GitLogRoot> | null {
  return parseRoots<GitLogRoot>(data);
}

function parseBranches(
  data: unknown,
): GitRootsResponse<GitBranchesRoot> | null {
  return parseRoots<GitBranchesRoot>(data);
}

function relativeCommitTime(timeMs: number): string {
  const minutes = Math.max(0, Math.floor((Date.now() - timeMs) / 60_000));
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${String(minutes)}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${String(hours)}h ago`;
  return `${String(Math.floor(hours / 24))}d ago`;
}

function CommitRow({ commit }: { commit: GitCommitLogEntry }) {
  return (
    <li className={styles.row}>
      <span className={styles.rowCopy}>
        <strong>{commit.message_first_line}</strong>
        <span>
          {commit.short_oid} · {commit.author_name}
        </span>
      </span>
      <span className={styles.rowMeta}>
        {relativeCommitTime(commit.time_ms)}
      </span>
    </li>
  );
}

function GitRootSection({
  daemonBase,
  projectId,
  status,
}: {
  daemonBase: string;
  projectId: string;
  status: GitStatusRoot;
}) {
  const rootQuery = encodeURIComponent(status.root);
  const log = useProjectResource(
    daemonBase,
    projectId,
    `/git/log?root=${rootQuery}&limit=${String(GIT_LOG_LIMIT)}&skip=0`,
    parseLog,
  );
  const branches = useProjectResource(
    daemonBase,
    projectId,
    `/git/branches?root=${rootQuery}`,
    parseBranches,
  );
  const branchesRoot =
    branches.resource.state === "ready"
      ? branches.resource.data.roots.at(0)
      : undefined;
  const commits =
    log.resource.state === "ready"
      ? log.resource.data.roots.at(0)?.commits ?? []
      : [];

  return (
    <Surface
      aria-label={`Git root ${status.root}`}
      className={styles.section}
      radius="card"
      variant="glass"
    >
      <h3 className={styles.sectionTitle}>{status.root}</h3>
      <dl className={styles.factGrid}>
        <Fact
          label="Branch"
          value={
            status.branch ?? (status.head_detached ? "detached HEAD" : "—")
          }
        />
        <Fact label="Staged" value={status.staged.length} />
        <Fact label="Unstaged" value={status.unstaged.length} />
        <Fact
          label="Branches"
          value={branchesRoot ? branchesRoot.branches.length : "—"}
        />
      </dl>
      <ResourceView
        errorText="Commit history is unavailable."
        resource={log.resource}
      >
        {() =>
          commits.length === 0 ? (
            <p className={styles.muted}>No commits yet.</p>
          ) : (
            <ul
              aria-label={`Recent commits in ${status.root}`}
              className={styles.list}
            >
              {commits.map((commit) => (
                <CommitRow commit={commit} key={commit.oid} />
              ))}
            </ul>
          )
        }
      </ResourceView>
    </Surface>
  );
}

function GitContent({
  daemonBase,
  projectId,
}: {
  daemonBase: string;
  projectId: string;
}) {
  const status = useProjectResource(
    daemonBase,
    projectId,
    "/git/status",
    parseStatus,
  );

  return (
    <ResourceView
      errorText="Git status is unavailable."
      resource={status.resource}
    >
      {(data) =>
        data.roots.length === 0 ? (
          <p className={styles.muted}>No git repositories in this project.</p>
        ) : (
          <>
            {data.roots.map((root) => (
              <GitRootSection
                daemonBase={daemonBase}
                key={root.root}
                projectId={projectId}
                status={root}
              />
            ))}
          </>
        )
      }
    </ResourceView>
  );
}

export function GitTab({ daemonBase, worker, onMutated }: GitTabProps) {
  return (
    <div className={styles.tabBody}>
      <WorkerGate onMutated={onMutated} worker={worker}>
        <GitContent daemonBase={daemonBase} projectId={worker.project_id} />
      </WorkerGate>
    </div>
  );
}

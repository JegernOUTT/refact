import { Copy } from "lucide-react";
import { useCallback, useMemo, useState } from "react";

import { IconButton, Spinner, Tooltip } from "../../../../components/ui";
import { useAppDispatch } from "../../../../hooks";
import { useCopyToClipboard } from "../../../../hooks/useCopyToClipboard";
import {
  useGetGitCommitDetailsQuery,
  type GitCommitDetails,
  type GitCommitFile,
} from "../../../../services/refact/gitRead";
import { ChangesTree } from "../ChangesTree";
import {
  flattenChangesTree,
  groupChildren,
  makeEntry,
  type ChangesTreeEntry,
} from "../changesTreeModel";
import { gitErrorText } from "../gitErrors";
import { openGitCommitDiff } from "../gitPanelSlice";
import { formatAbsoluteDate, splitMessage } from "./commitFormat";
import type { RefLabelContext } from "./refLabels";
import { RefPills } from "./RefPills";
import styles from "./GitLog.module.css";

const MAX_LISTED_BRANCHES = 5;
const NO_COLLAPSED: ReadonlySet<string> = new Set();
const NO_INCLUSION = {
  excludedChanges: new Set<string>(),
  includedUnversioned: new Set<string>(),
};

export type CommitDetailsPaneProps = {
  root: string;
  oid: string | null;
  isLoaded: (oid: string) => boolean;
  onSelectCommit: (oid: string) => void;
  refContext?: RefLabelContext;
};

function joinPath(root: string, path: string): string {
  return `${root.replace(/[/\\]+$/, "")}/${path}`;
}

function branchesLabel(branches: string[], truncated: boolean): string {
  const listed = branches.slice(0, MAX_LISTED_BRANCHES).join(", ");
  const rest = branches.length - MAX_LISTED_BRANCHES;
  const more = truncated ? "+" : "";
  if (rest > 0) return `${listed} and ${rest}${more} more`;
  return truncated ? `${listed} and more` : listed;
}

function FileStats({ file }: { file: GitCommitFile | undefined }) {
  if (!file) return null;
  if (file.binary) return <span className={styles.fileStat}>binary</span>;
  return (
    <span className={styles.fileStat}>
      {file.additions > 0 ? (
        <span data-tone="added">+{file.additions}</span>
      ) : null}
      {file.deletions > 0 ? (
        <span data-tone="deleted">−{file.deletions}</span>
      ) : null}
    </span>
  );
}

function CommitFiles({ details }: { details: GitCommitDetails }) {
  const dispatch = useAppDispatch();
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(NO_COLLAPSED);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const byPath = useMemo(
    () => new Map(details.files.map((file) => [file.path, file])),
    [details.files],
  );
  const rows = useMemo(() => {
    const entries = details.files.map((file) =>
      makeEntry(
        "change",
        file.path,
        joinPath(details.root, file.path),
        file.status,
        file.old_path,
      ),
    );
    return flattenChangesTree(
      groupChildren("changes", entries, true),
      collapsed,
    );
  }, [collapsed, details.files, details.root]);

  const openDiff = useCallback(
    (entry: ChangesTreeEntry) => {
      dispatch(
        openGitCommitDiff({
          root: details.root,
          path: entry.path,
          old_path: entry.oldPath,
          status: entry.status,
          commit: { oid: details.oid, short_oid: details.short_oid },
          parent_oid: details.parents.length > 0 ? details.parents[0] : null,
        }),
      );
    },
    [details, dispatch],
  );

  if (details.files.length === 0) {
    return <p className={styles.mutedText}>No file changes.</p>;
  }

  return (
    <div className={styles.filesTree}>
      <ChangesTree
        checkable={false}
        collapsed={collapsed}
        groupByDirectory
        inclusion={NO_INCLUSION}
        label="Changed files"
        onSelect={setSelectedId}
        onSetExpanded={(id, expanded) =>
          setCollapsed((current) => {
            const next = new Set(current);
            if (expanded) next.delete(id);
            else next.add(id);
            return next;
          })
        }
        onShowDiff={openDiff}
        onToggleInclude={() => undefined}
        renderContextMenu={() => null}
        renderEntryMeta={(entry) => <FileStats file={byPath.get(entry.path)} />}
        rows={rows}
        selectedId={selectedId}
      />
    </div>
  );
}

function Details({
  details,
  isLoaded,
  onSelectCommit,
  refContext,
}: {
  details: GitCommitDetails;
  isLoaded: (oid: string) => boolean;
  onSelectCommit: (oid: string) => void;
  refContext?: RefLabelContext;
}) {
  const copy = useCopyToClipboard();
  const { subject, body } = splitMessage(details.message);
  const committerDiffers =
    details.committer_name !== details.author_name ||
    details.committer_email !== details.author_email;
  const additions = details.files.reduce(
    (sum, file) => sum + file.additions,
    0,
  );
  const deletions = details.files.reduce(
    (sum, file) => sum + file.deletions,
    0,
  );

  return (
    <div className={styles.details}>
      <section className={styles.detailsSection} aria-label="Commit message">
        <h2 className={styles.detailsSubject}>{subject}</h2>
        {body ? <pre className={styles.detailsBody}>{body}</pre> : null}
      </section>
      <section className={styles.detailsSection} aria-label="Commit metadata">
        <div className={styles.metaRow}>
          <code className={styles.hash} title={details.oid}>
            {details.short_oid}
          </code>
          <Tooltip content="Copy commit hash">
            <IconButton
              aria-label="Copy commit hash"
              icon={Copy}
              onClick={() => copy(details.oid)}
              size="sm"
              variant="ghost"
            />
          </Tooltip>
          <RefPills context={refContext} refs={details.refs} />
        </div>
        <div className={styles.metaRow}>
          <span>
            {details.author_name}{" "}
            <span className={styles.mutedText}>
              &lt;{details.author_email}&gt;
            </span>
          </span>
        </div>
        <div className={styles.mutedText}>
          {formatAbsoluteDate(details.author_time_ms)}
        </div>
        {committerDiffers ? (
          <div className={styles.mutedText}>
            Committed by {details.committer_name} on{" "}
            {formatAbsoluteDate(details.committer_time_ms)}
          </div>
        ) : null}
        {details.parents.length > 0 ? (
          <div className={styles.metaRow}>
            <span className={styles.mutedText}>
              {details.parents.length > 1 ? "Parents" : "Parent"}
            </span>
            {details.parents.map((parent) =>
              isLoaded(parent) ? (
                <button
                  className={styles.parentLink}
                  key={parent}
                  onClick={() => onSelectCommit(parent)}
                  title={parent}
                  type="button"
                >
                  {parent.slice(0, details.short_oid.length)}
                </button>
              ) : (
                <code className={styles.hash} key={parent} title={parent}>
                  {parent.slice(0, details.short_oid.length)}
                </code>
              ),
            )}
          </div>
        ) : (
          <div className={styles.mutedText}>Root commit</div>
        )}
        {details.branches.length > 0 ? (
          <div className={styles.mutedText} title={details.branches.join(", ")}>
            In {details.branches.length}
            {details.branches_truncated === true ? "+" : ""}{" "}
            {details.branches.length === 1 &&
            details.branches_truncated !== true
              ? "branch"
              : "branches"}
            :{" "}
            {branchesLabel(
              details.branches,
              details.branches_truncated === true,
            )}
          </div>
        ) : null}
      </section>
      <section
        className={styles.detailsSection}
        data-grow="true"
        aria-label="Changed files"
      >
        <div className={styles.filesHeader}>
          <span>
            {details.files.length}{" "}
            {details.files.length === 1 ? "file" : "files"} changed
          </span>
          <span className={styles.fileStat}>
            <span data-tone="added">+{additions}</span>
            <span data-tone="deleted">−{deletions}</span>
          </span>
        </div>
        {details.files_truncated ? (
          <p className={styles.mutedText} role="note">
            Too many files; the list is truncated.
          </p>
        ) : null}
        <CommitFiles details={details} key={details.oid} />
      </section>
    </div>
  );
}

export function CommitDetailsPane({
  root,
  oid,
  isLoaded,
  onSelectCommit,
  refContext,
}: CommitDetailsPaneProps) {
  const query = useGetGitCommitDetailsQuery(
    { root, oid: oid ?? "" },
    { skip: !oid },
  );

  if (!oid) {
    return (
      <div className={styles.paneState}>
        <p className={styles.mutedText}>Select a commit to see its details.</p>
      </div>
    );
  }
  if (query.error) {
    return (
      <p className={styles.errorText} role="alert">
        {gitErrorText(query.error).message}
      </p>
    );
  }
  if (!query.data || query.data.oid !== oid) {
    return (
      <div className={styles.paneState}>
        <Spinner label="Loading commit details" size="sm" />
      </div>
    );
  }
  return (
    <Details
      details={query.data}
      isLoaded={isLoaded}
      onSelectCommit={onSelectCommit}
      refContext={refContext}
    />
  );
}

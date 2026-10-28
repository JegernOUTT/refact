import { ContextMenu } from "@radix-ui/themes";
import { RefreshCw } from "lucide-react";
import {
  forwardRef,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  type KeyboardEvent,
  type UIEvent,
} from "react";

import {
  Button,
  Combobox,
  FieldSelect,
  IconButton,
  Spinner,
  Tooltip,
  VirtualList,
  type VirtualListHandle,
} from "../../../../components/ui";
import type { GitCommitLogEntry } from "../../../../services/refact/gitRead";
import {
  ContextMenuActions,
  type GitMenuAction,
} from "../branches/menuActions";
import { gitErrorText } from "../gitErrors";
import type { BranchRef } from "./branchesTreeModel";
import { CommitGraphCell } from "./CommitGraphCell";
import type { GraphLayout } from "./commitGraphLayout";
import { formatAbsoluteDate, formatCommitDate } from "./commitFormat";
import {
  branchFilterLabel,
  type BranchFilter,
  type LogFilters,
} from "./logFilters";
import type { RefLabelContext } from "./refLabels";
import { RefPills } from "./RefPills";
import { useDebouncedInput } from "./useDebouncedValue";
import styles from "./GitLog.module.css";

const PAGE_STEP = 20;
const LOAD_MORE_THRESHOLD_PX = 400;
const BRANCH_ALL = "__all__";
const BRANCH_HEAD = "__head__";

export type CommitsPaneProps = {
  commits: GitCommitLogEntry[];
  graph: GraphLayout | null;
  filters: LogFilters;
  refs: BranchRef[];
  refContext?: RefLabelContext;
  selectedOid: string | null;
  isLoading: boolean;
  isFetching: boolean;
  loadingMore: boolean;
  hasMore: boolean;
  scanTruncated: boolean;
  error: unknown;
  pageError: unknown;
  onSelect: (oid: string) => void;
  onFiltersChange: (patch: Partial<LogFilters>) => void;
  onLoadMore: () => void;
  onRefresh: () => void;
  commitActions?: (commit: GitCommitLogEntry) => GitMenuAction[];
};

function branchValue(branch: BranchFilter): string {
  if (branch.kind === "all") return BRANCH_ALL;
  if (branch.kind === "head") return BRANCH_HEAD;
  return branch.name;
}

function branchFromValue(value: string): BranchFilter {
  if (value === BRANCH_ALL) return { kind: "all" };
  if (value === BRANCH_HEAD) return { kind: "head" };
  return { kind: "ref", name: value };
}

function FilterInput({
  label,
  placeholder,
  value,
  onCommit,
}: {
  label: string;
  placeholder: string;
  value: string;
  onCommit: (value: string) => void;
}) {
  const [draft, setDraft] = useDebouncedInput(value, onCommit);
  return (
    <input
      aria-label={label}
      className={styles.filterInput}
      onChange={(event) => setDraft(event.currentTarget.value)}
      placeholder={placeholder}
      type="search"
      value={draft}
    />
  );
}

function AuthorFilter({
  value,
  authors,
  onCommit,
}: {
  value: string;
  authors: string[];
  onCommit: (value: string) => void;
}) {
  const [draft, setDraft] = useDebouncedInput(value, onCommit);
  const items = useMemo(
    () => authors.map((author) => ({ value: author })),
    [authors],
  );
  return (
    <div className={styles.authorFilter}>
      <Combobox
        aria-label="Filter by user"
        items={items}
        onSelect={(item) => {
          setDraft(item.value);
          onCommit(item.value);
        }}
        onValueChange={setDraft}
        placeholder="User"
        value={draft}
      />
    </div>
  );
}

type CommitRowProps = {
  commit: GitCommitLogEntry;
  graph: GraphLayout | null;
  index: number;
  rowId: string;
  selected: boolean;
  onSelect: (oid: string) => void;
  refContext?: RefLabelContext;
  commitActions?: (commit: GitCommitLogEntry) => GitMenuAction[];
};

function CommitRow({ commitActions, ...props }: CommitRowProps) {
  if (!commitActions) return <CommitRowContent {...props} />;
  return (
    <ContextMenu.Root modal={false}>
      <ContextMenu.Trigger>
        <CommitRowContent {...props} />
      </ContextMenu.Trigger>
      <ContextMenu.Content size="1">
        <ContextMenuActions actions={commitActions(props.commit)} />
      </ContextMenu.Content>
    </ContextMenu.Root>
  );
}

const CommitRowContent = forwardRef<
  HTMLDivElement,
  Omit<CommitRowProps, "commitActions">
>(function CommitRowContent(
  { commit, graph, index, rowId, selected, onSelect, refContext, ...rest },
  ref,
) {
  const graphRow = graph?.rows[index];
  const absolute = formatAbsoluteDate(commit.time_ms);
  return (
    <div
      {...rest}
      aria-selected={selected}
      className={styles.commitRow}
      data-oid={commit.oid}
      data-selected={selected ? "true" : undefined}
      id={rowId}
      onClick={() => onSelect(commit.oid)}
      ref={ref}
      role="option"
      title={`${commit.short_oid} · ${commit.message_first_line}`}
    >
      <span className={styles.subjectCell}>
        {graphRow ? (
          <CommitGraphCell row={graphRow} width={graphRow.width} />
        ) : null}
        <RefPills context={refContext} refs={commit.refs} />
        <span className={styles.subjectText}>{commit.message_first_line}</span>
      </span>
      <span className={styles.authorCell} title={commit.author_email}>
        {commit.author_name}
      </span>
      <span className={styles.dateCell} title={absolute}>
        {formatCommitDate(commit.time_ms)}
      </span>
      <span className={styles.hashCell}>{commit.short_oid}</span>
    </div>
  );
});

export function CommitsPane({
  commits,
  graph,
  filters,
  refs,
  refContext,
  selectedOid,
  isLoading,
  isFetching,
  loadingMore,
  hasMore,
  scanTruncated,
  error,
  pageError,
  onSelect,
  onFiltersChange,
  onLoadMore,
  onRefresh,
  commitActions,
}: CommitsPaneProps) {
  const listId = useId();
  const listRef = useRef<VirtualListHandle>(null);
  const focusRef = useRef<HTMLDivElement>(null);
  const selectedIndex = commits.findIndex(
    (commit) => commit.oid === selectedOid,
  );
  const rowId = (oid: string) => `${listId}-${oid}`;
  const authors = useMemo(
    () =>
      [...new Set(commits.map((commit) => commit.author_name))].sort((a, b) =>
        a.localeCompare(b),
      ),
    [commits],
  );
  const branchOptions = useMemo(() => {
    const options = [
      { value: BRANCH_ALL, label: "All" },
      { value: BRANCH_HEAD, label: "HEAD" },
      ...refs.map((ref) => ({ value: ref.name, label: ref.name })),
    ];
    if (
      filters.branch.kind === "ref" &&
      !refs.some(
        (ref) =>
          filters.branch.kind === "ref" && ref.name === filters.branch.name,
      )
    ) {
      options.push({
        value: filters.branch.name,
        label: branchFilterLabel(filters.branch),
      });
    }
    return options;
  }, [filters.branch, refs]);

  useEffect(() => {
    if (selectedIndex >= 0) {
      listRef.current?.scrollIntoView({ index: selectedIndex });
    }
  }, [selectedIndex]);

  const handleKeyDown = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      if (commits.length === 0) return;
      const index = selectedIndex;
      const last = commits.length - 1;
      const targets: Record<string, number> = {
        ArrowDown: index < 0 ? 0 : Math.min(index + 1, last),
        ArrowUp: Math.max(index - 1, 0),
        Home: 0,
        End: last,
        PageDown: Math.min(Math.max(index, 0) + PAGE_STEP, last),
        PageUp: Math.max(index - PAGE_STEP, 0),
      };
      if (!(event.key in targets)) return;
      event.preventDefault();
      const next = targets[event.key];
      onSelect(commits[next].oid);
      if (next === last && hasMore) onLoadMore();
    },
    [commits, hasMore, onLoadMore, onSelect, selectedIndex],
  );

  const handleScroll = (event: UIEvent<HTMLDivElement>) => {
    const target = event.currentTarget;
    const remaining =
      target.scrollHeight - target.scrollTop - target.clientHeight;
    if (hasMore && remaining < LOAD_MORE_THRESHOLD_PX) onLoadMore();
  };

  // Kept outside the listbox: a listbox may only own options, so the paging
  // status (and its button) is a sibling region instead of a fake row.
  const statusRow =
    hasMore || loadingMore || pageError ? (
      <div className={styles.loadingRow}>
        {loadingMore ? (
          <Spinner label="Loading more commits" size="sm" />
        ) : (
          <Button onClick={onLoadMore} size="sm" type="button" variant="ghost">
            {pageError ? "Retry loading commits" : "Load more"}
          </Button>
        )}
      </div>
    ) : scanTruncated ? (
      <div className={styles.loadingRow} role="note">
        Search stopped early; refine the filters to find older commits.
      </div>
    ) : null;

  return (
    <div className={styles.commitsPane}>
      <div className={styles.filterBar} role="toolbar" aria-label="Log filters">
        <FilterInput
          label="Filter by text or hash"
          onCommit={(text) => onFiltersChange({ text })}
          placeholder="Text or hash"
          value={filters.text}
        />
        <div className={styles.branchFilter}>
          <FieldSelect
            aria-label="Branch filter"
            onChange={(value) =>
              onFiltersChange({ branch: branchFromValue(value) })
            }
            options={branchOptions}
            value={branchValue(filters.branch)}
          />
        </div>
        <AuthorFilter
          authors={authors}
          onCommit={(author) => onFiltersChange({ author })}
          value={filters.author}
        />
        <FilterInput
          label="Filter by path"
          onCommit={(path) => onFiltersChange({ path })}
          placeholder="Path"
          value={filters.path}
        />
        <Tooltip content="Refresh">
          <IconButton
            aria-label="Refresh log"
            icon={RefreshCw}
            loading={isFetching}
            onClick={onRefresh}
            size="sm"
            variant="ghost"
          />
        </Tooltip>
      </div>
      {error ? (
        <p className={styles.errorText} role="alert">
          {gitErrorText(error).message}
        </p>
      ) : isLoading ? (
        <div className={styles.paneState}>
          <Spinner label="Loading commits" size="sm" />
        </div>
      ) : (
        <div
          aria-activedescendant={
            selectedIndex >= 0 ? rowId(commits[selectedIndex].oid) : undefined
          }
          aria-label="Commits"
          className={styles.commitList}
          data-graph={graph ? "true" : undefined}
          onKeyDown={handleKeyDown}
          ref={focusRef}
          role="listbox"
          tabIndex={0}
        >
          <VirtualList
            emptyMessage={
              filters.text || filters.author || filters.path
                ? "No commits match the filters."
                : "No commits yet"
            }
            getItemKey={(item) => item.oid}
            height="100%"
            items={commits}
            listRef={listRef}
            onListScroll={handleScroll}
            renderItem={(commit, index) => (
              <CommitRow
                commit={commit}
                commitActions={commitActions}
                graph={graph}
                index={index}
                onSelect={(oid) => {
                  onSelect(oid);
                  focusRef.current?.focus();
                }}
                refContext={refContext}
                rowId={rowId(commit.oid)}
                selected={commit.oid === selectedOid}
              />
            )}
          />
        </div>
      )}
      {!error && !isLoading && commits.length > 0 ? statusRow : null}
    </div>
  );
}

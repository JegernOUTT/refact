import { Badge, Button, Spinner } from "../../../components/ui";
import { Checkbox } from "../../../components/Checkbox";
import type {
  GitFileChange,
  GitStatusRoot,
} from "../../../services/refact/gitRead";
import styles from "./GitPanel.module.css";

export type SelectedGitFile = {
  path: string;
  staged: boolean;
};

type StatusListProps = {
  status?: GitStatusRoot;
  selected: SelectedGitFile | null;
  pendingPath: string | null;
  isLoading: boolean;
  error: string | null;
  onSelect: (file: SelectedGitFile) => void;
  onStage: (change: GitFileChange) => void;
  onUnstage: (change: GitFileChange) => void;
  onRefresh: () => void;
};

function StatusGroup({
  title,
  changes,
  staged,
  selected,
  pendingPath,
  onSelect,
  onToggle,
}: {
  title: string;
  changes: GitFileChange[];
  staged: boolean;
  selected: SelectedGitFile | null;
  pendingPath: string | null;
  onSelect: (file: SelectedGitFile) => void;
  onToggle: (change: GitFileChange) => void;
}) {
  return (
    <section className={styles.statusGroup} aria-label={title}>
      <header className={styles.statusGroupHeader}>
        <span>{title}</span>
        <Badge tone="muted" size="xs">
          {changes.length}
        </Badge>
      </header>
      {changes.length === 0 ? (
        <p className={styles.emptyText}>No {title.toLowerCase()} changes.</p>
      ) : (
        <ul className={styles.statusList}>
          {changes.map((change) => {
            const isSelected =
              selected?.path === change.relative_path &&
              selected.staged === staged;
            const pending = pendingPath === change.relative_path;
            const action = staged ? "Unstage" : "Stage";
            return (
              <li
                key={`${staged ? "staged" : "unstaged"}-${
                  change.relative_path
                }`}
              >
                <div
                  className={styles.statusRow}
                  data-selected={isSelected || undefined}
                >
                  <Checkbox
                    checked={staged}
                    disabled={pendingPath !== null}
                    aria-label={`${action} ${change.relative_path}`}
                    onCheckedChange={() => onToggle(change)}
                  />
                  <button
                    type="button"
                    className={styles.fileButton}
                    aria-pressed={isSelected}
                    onClick={() =>
                      onSelect({ path: change.relative_path, staged })
                    }
                  >
                    <span className={styles.filePath}>
                      {change.relative_path}
                    </span>
                    <Badge tone={staged ? "success" : "warning"} size="xs">
                      {change.status}
                    </Badge>
                  </button>
                  {pending ? (
                    <Spinner size="sm" label={`${action} pending`} />
                  ) : null}
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

export function StatusList({
  status,
  selected,
  pendingPath,
  isLoading,
  error,
  onSelect,
  onStage,
  onUnstage,
  onRefresh,
}: StatusListProps) {
  if (isLoading && !status) {
    return (
      <div className={styles.loadingBlock}>
        <Spinner label="Loading Git status" />
      </div>
    );
  }

  if (!status) {
    return (
      <div className={styles.inlineState}>
        <p>{error ?? "No Git repository status is available."}</p>
        <Button type="button" variant="soft" size="sm" onClick={onRefresh}>
          Retry
        </Button>
      </div>
    );
  }

  return (
    <div className={styles.statusGroups}>
      {error ? (
        <p className={styles.errorText} role="alert">
          {error}
        </p>
      ) : null}
      <StatusGroup
        title="Staged"
        changes={status.staged}
        staged
        selected={selected}
        pendingPath={pendingPath}
        onSelect={onSelect}
        onToggle={onUnstage}
      />
      <StatusGroup
        title="Unstaged"
        changes={status.unstaged}
        staged={false}
        selected={selected}
        pendingPath={pendingPath}
        onSelect={onSelect}
        onToggle={onStage}
      />
    </div>
  );
}

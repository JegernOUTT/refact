import { useState } from "react";

import { Button, Field, FieldTextarea } from "../../../components/ui";
import {
  type GitFileChange,
  useCommitGitChangesMutation,
} from "../../../services/refact/gitRead";
import { worktreeErrorText } from "../../Worktrees/worktreeError";
import styles from "./GitPanel.module.css";

type CommitBoxProps = {
  root: string;
  stagedChanges: GitFileChange[];
  onCommitted: (shortOid: string) => void;
};

function first<T>(values: T[]): T | undefined {
  return values.length > 0 ? values[0] : undefined;
}

export function CommitBox({
  root,
  stagedChanges,
  onCommitted,
}: CommitBoxProps) {
  const [message, setMessage] = useState("");
  const [commitChanges, commitState] = useCommitGitChangesMutation();
  const [error, setError] = useState<string | null>(null);
  const messageId = "git-commit-message";
  const canCommit = stagedChanges.length > 0 && message.trim().length > 0;

  const handleCommit = async () => {
    setError(null);
    try {
      const response = await commitChanges({
        root,
        body: {
          commits: [
            {
              root,
              commit_message: message.trim(),
              staged_changes: stagedChanges.map(
                ({ relative_path, absolute_path, status }) => ({
                  relative_path,
                  absolute_path,
                  status,
                }),
              ),
              unstaged_changes: [],
            },
          ],
        },
      }).unwrap();
      const applied = first(response.commits_applied);
      if (!applied) {
        const firstError = first(response.error_log)?.error_message;
        throw new Error(firstError ?? "The commit was not created.");
      }
      setMessage("");
      onCommitted(applied.commit_oid.slice(0, 8));
    } catch (commitError) {
      setError(worktreeErrorText(commitError));
    }
  };

  return (
    <section className={styles.section} aria-labelledby="git-commit-heading">
      <header className={styles.sectionHeader}>
        <div>
          <h2 id="git-commit-heading">Commit</h2>
          <p>
            {stagedChanges.length} staged file
            {stagedChanges.length === 1 ? "" : "s"} in the active root.
          </p>
        </div>
      </header>
      <Field
        label="Commit message"
        htmlFor={messageId}
        error={error ?? undefined}
      >
        <FieldTextarea
          id={messageId}
          value={message}
          onChange={setMessage}
          rows={4}
          placeholder="Describe the staged changes"
          disabled={commitState.isLoading}
        />
      </Field>
      <div className={styles.actionsRow}>
        <Button
          type="button"
          variant="primary"
          size="sm"
          loading={commitState.isLoading}
          disabled={!canCommit || commitState.isLoading}
          onClick={() => void handleCommit()}
        >
          Commit staged changes
        </Button>
      </div>
    </section>
  );
}

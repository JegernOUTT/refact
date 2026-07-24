import { useState } from "react";
import { Sparkles } from "lucide-react";

import {
  Button,
  Field,
  FieldTextarea,
  IconButton,
  Tooltip,
} from "../../../components/ui";
import {
  type GitFileChange,
  useCommitGitChangesMutation,
  useGenerateCommitMessageMutation,
  useGetGitDiffQuery,
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
  const [generateCommitMessage, generateState] =
    useGenerateCommitMessageMutation();
  const stagedDiffQuery = useGetGitDiffQuery(
    { root, staged: true },
    { skip: stagedChanges.length === 0 },
  );
  const [error, setError] = useState<string | null>(null);
  const messageId = "git-commit-message";
  const canCommit = stagedChanges.length > 0 && message.trim().length > 0;
  const isGenerating = stagedDiffQuery.isFetching || generateState.isLoading;

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

  const handleGenerate = async () => {
    setError(null);
    try {
      const diffResponse = await stagedDiffQuery.refetch().unwrap();
      const patch = first(diffResponse.roots)?.patch;
      if (!patch) {
        throw new Error("The staged diff is empty.");
      }
      const steeringText = message.trim();
      const generated = await generateCommitMessage({
        diff: patch,
        ...(steeringText ? { text: steeringText } : {}),
      }).unwrap();
      setMessage(generated);
    } catch (generateError) {
      setError(worktreeErrorText(generateError));
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
        <div className={styles.commitMessageRow}>
          <FieldTextarea
            id={messageId}
            value={message}
            onChange={setMessage}
            rows={4}
            placeholder="Describe the staged changes"
            disabled={commitState.isLoading || isGenerating}
          />
          <Tooltip content="Generate commit message">
            <span className={styles.generateButtonWrap}>
              <IconButton
                aria-label="Generate commit message"
                icon={Sparkles}
                loading={isGenerating}
                disabled={stagedChanges.length === 0 || isGenerating}
                onClick={() => void handleGenerate()}
                size="sm"
                variant="plain"
              />
            </span>
          </Tooltip>
        </div>
      </Field>
      <div className={styles.actionsRow}>
        <Button
          type="button"
          variant="primary"
          size="sm"
          loading={commitState.isLoading}
          disabled={!canCommit || commitState.isLoading || isGenerating}
          onClick={() => void handleCommit()}
        >
          Commit staged changes
        </Button>
      </div>
    </section>
  );
}

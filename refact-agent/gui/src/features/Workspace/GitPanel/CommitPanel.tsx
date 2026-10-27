import { useId, useRef, useState, type KeyboardEvent, type Ref } from "react";
import { Sparkles } from "lucide-react";

import { Checkbox } from "../../../components/Checkbox";
import {
  Button,
  FieldTextarea,
  IconButton,
  Tooltip,
} from "../../../components/ui";
import {
  useCommitGitPathsMutation,
  useGenerateCommitMessageMutation,
  useLazyGetGitDiffQuery,
  useLazyGetGitLogQuery,
  type GitSkippedPath,
} from "../../../services/refact/gitRead";
import { SkippedPaths } from "./GitActionDialogs";
import { gitErrorText, type GitErrorView } from "./gitErrors";
import styles from "./GitDock.module.css";

const MAX_GENERATION_DIFF_BYTES = 200 * 1024;

function capDiff(patch: string): string {
  const bytes = new TextEncoder().encode(patch);
  if (bytes.length <= MAX_GENERATION_DIFF_BYTES) return patch;
  return new TextDecoder()
    .decode(bytes.slice(0, MAX_GENERATION_DIFF_BYTES))
    .replace(/\uFFFD$/u, "");
}

export type CommitPanelProps = {
  root: string;
  includedCount: number;
  totalCount: number;
  paths: string[];
  amendAvailable: boolean;
  /** Why Amend is unavailable; shown as its tooltip. */
  amendBlockedReason?: string | null;
  canPush: boolean;
  messageRef?: Ref<HTMLTextAreaElement>;
  onCommitted: () => void;
  onPushRequested: () => void | Promise<void>;
};

type CommitResult = {
  text: string;
  skipped: GitSkippedPath[];
};

export function CommitPanel({
  root,
  includedCount,
  totalCount,
  paths,
  amendAvailable,
  amendBlockedReason,
  canPush,
  messageRef,
  onCommitted,
  onPushRequested,
}: CommitPanelProps) {
  const messageId = useId();
  const [message, setMessage] = useState("");
  const [amend, setAmend] = useState(false);
  const draftBeforeAmend = useRef<string | null>(null);
  const [error, setError] = useState<GitErrorView | null>(null);
  const [result, setResult] = useState<CommitResult | null>(null);
  const [commit, commitState] = useCommitGitPathsMutation();
  const [generate, generateState] = useGenerateCommitMessageMutation();
  const [fetchDiff, diffState] = useLazyGetGitDiffQuery();
  const [fetchLog, logState] = useLazyGetGitLogQuery();
  const generating = generateState.isLoading || diffState.isFetching;
  const busy = commitState.isLoading || generating;
  const canCommit =
    message.trim().length > 0 && (includedCount > 0 || amend) && !busy;

  // Leave amend mode if it becomes unavailable (e.g. a merge started).
  if (amend && !amendAvailable) setAmend(false);

  const toggleAmend = async (checked: boolean) => {
    setAmend(checked);
    setError(null);
    if (!checked) {
      if (draftBeforeAmend.current !== null) {
        setMessage(draftBeforeAmend.current);
        draftBeforeAmend.current = null;
      }
      return;
    }
    if (message.trim().length > 0) return;
    draftBeforeAmend.current = message;
    try {
      const log = await fetchLog({ root, limit: 1, skip: 0 }).unwrap();
      const head = log.roots.at(0)?.commits.at(0);
      if (head) setMessage(head.message.replace(/\s+$/u, ""));
    } catch (fetchError) {
      setError(gitErrorText(fetchError));
    }
  };

  const handleGenerate = async () => {
    setError(null);
    try {
      const diff = await fetchDiff({ root, paths, against: "head" }).unwrap();
      const patch = diff.roots[0]?.patch ?? "";
      if (!patch.trim()) throw new Error("The included changes have no diff.");
      const steering = message.trim();
      const generated = await generate({
        diff: capDiff(patch),
        ...(steering ? { text: steering } : {}),
      }).unwrap();
      setMessage(generated);
    } catch (generateError) {
      setError(gitErrorText(generateError));
    }
  };

  const handleCommit = async (pushAfter = false) => {
    if (!canCommit) return;
    setError(null);
    setResult(null);
    try {
      const response = await commit({
        root,
        message: message.trim(),
        paths,
        ...(amend ? { amend: true } : {}),
      }).unwrap();
      setMessage("");
      setAmend(false);
      draftBeforeAmend.current = null;
      setResult({
        text: `Committed ${response.short_oid}: ${response.summary}`,
        skipped: response.skipped,
      });
      onCommitted();
      if (pushAfter) await onPushRequested();
    } catch (commitError) {
      setError(gitErrorText(commitError));
    }
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      void handleCommit();
    }
  };

  return (
    <section aria-label="Commit" className={styles.commitPanel}>
      <div className={styles.commitOptions}>
        {!amendAvailable && amendBlockedReason ? (
          <Tooltip content={amendBlockedReason}>
            <span className={styles.disabledTooltipHost}>
              <Checkbox checked={false} disabled title={amendBlockedReason}>
                Amend
              </Checkbox>
            </span>
          </Tooltip>
        ) : (
          <Checkbox
            checked={amend}
            disabled={!amendAvailable || busy || logState.isFetching}
            onCheckedChange={(checked) => void toggleAmend(checked === true)}
          >
            Amend
          </Checkbox>
        )}
        <Tooltip content="Generate commit message">
          <IconButton
            aria-label="Generate commit message"
            disabled={includedCount === 0 || busy}
            icon={Sparkles}
            loading={generating}
            onClick={() => void handleGenerate()}
            size="sm"
            variant="ghost"
          />
        </Tooltip>
      </div>
      <label className={styles.srOnly} htmlFor={messageId}>
        Commit message
      </label>
      <FieldTextarea
        className={styles.commitMessage}
        disabled={busy}
        id={messageId}
        ref={messageRef}
        onChange={setMessage}
        onKeyDown={handleKeyDown}
        placeholder="Commit Message"
        rows={4}
        value={message}
      />
      {error ? (
        <div className={styles.errorText} role="alert">
          <span>{error.message}</span>
          {error.output ? (
            <details className={styles.hookOutput}>
              <summary>Hook output</summary>
              <pre className="scrollX">{error.output}</pre>
            </details>
          ) : null}
        </div>
      ) : null}
      {result ? (
        <div className={styles.successText} role="status">
          <span>{result.text}</span>
          <SkippedPaths skipped={result.skipped} />
        </div>
      ) : null}
      <div className={styles.commitFooter}>
        <span className={styles.mutedText}>
          {includedCount} of {totalCount} {totalCount === 1 ? "file" : "files"}{" "}
          included
        </span>
        <span className={styles.commitButtons}>
          <Button
            disabled={!canCommit}
            loading={commitState.isLoading}
            onClick={() => void handleCommit()}
            size="sm"
            variant="primary"
          >
            {amend ? "Amend Commit" : "Commit"}
          </Button>
          <Button
            disabled={!canCommit || !canPush}
            onClick={() => void handleCommit(true)}
            size="sm"
            variant="soft"
          >
            {amend ? "Amend Commit and Push…" : "Commit and Push…"}
          </Button>
        </span>
      </div>
    </section>
  );
}

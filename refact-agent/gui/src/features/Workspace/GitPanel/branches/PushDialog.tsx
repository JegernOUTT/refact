import { useId, useState, type FormEvent } from "react";

import { Checkbox } from "../../../../components/Checkbox";
import {
  Badge,
  Button,
  Dialog,
  FieldSelect,
  Spinner,
} from "../../../../components/ui";
import { useGetGitLogQuery } from "../../../../services/refact/gitRead";
import type { GitErrorView } from "../gitErrors";
import { gitErrorText } from "../gitErrors";
import { remoteOfRef } from "./branchName";
import { GitErrorDetails } from "./GitFeedback";
import styles from "./Branches.module.css";

const DIALOG_WIDTH = "calc(var(--rf-space-6) * 18)";

export type PushTarget = {
  remote: string;
  destination: string;
  setUpstream: boolean;
  forceWithLease: boolean;
};

export type PushDialogProps = {
  root: string;
  branch: string;
  upstream: string | null;
  ahead: number | null;
  remotes: readonly string[];
  setUpstreamMode: boolean;
  /** Set when the engine refused an implicit push to a differently named upstream. */
  upstreamMismatch?: boolean;
  busy: boolean;
  error: GitErrorView | null;
  onCancel: () => void;
  onConfirm: (target: PushTarget) => void;
};

function defaultRemote(
  upstream: string | null,
  remotes: readonly string[],
): string {
  const fromUpstream = upstream ? remoteOfRef(upstream, remotes) : null;
  if (fromUpstream) return fromUpstream.remote;
  if (remotes.includes("origin")) return "origin";
  return remotes.at(0) ?? "origin";
}

function OutgoingCommits({ root, count }: { root: string; count: number }) {
  const log = useGetGitLogQuery(
    { root, limit: count, skip: 0, scope: "head" },
    { skip: count === 0 },
  );
  if (count === 0) {
    return (
      <p className={styles.mutedText} role="status">
        No outgoing commits.
      </p>
    );
  }
  if (log.isLoading)
    return <Spinner label="Loading outgoing commits" size="sm" />;
  if (log.error) {
    return (
      <p className={styles.mutedText} role="alert">
        {gitErrorText(log.error).message}
      </p>
    );
  }
  const commits = log.data?.roots.at(0)?.commits ?? [];
  return (
    <ul aria-label="Outgoing commits" className={styles.outgoingList}>
      {commits.map((commit) => (
        <li className={styles.outgoingRow} key={commit.oid}>
          <span className={styles.outgoingSubject}>
            {commit.message_first_line}
          </span>
          <span className={styles.outgoingMeta}>{commit.author_name}</span>
          <code className={styles.outgoingMeta}>{commit.short_oid}</code>
        </li>
      ))}
    </ul>
  );
}

export function PushDialog({
  root,
  branch,
  upstream,
  ahead,
  remotes,
  setUpstreamMode,
  upstreamMismatch = false,
  busy,
  error,
  onCancel,
  onConfirm,
}: PushDialogProps) {
  const formId = useId();
  const [remote, setRemote] = useState(() => defaultRemote(upstream, remotes));
  const [force, setForce] = useState(false);
  const tracked =
    upstream && !setUpstreamMode ? remoteOfRef(upstream, remotes) : null;
  const targetRemote = tracked?.remote ?? remote;
  const targetBranch = tracked?.branch ?? branch;
  const isNew = tracked === null;
  const remoteOptions = (remotes.length > 0 ? remotes : [remote]).map(
    (name) => ({ value: name, label: name }),
  );

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (busy) return;
    onConfirm({
      remote: targetRemote,
      destination: `${targetRemote}/${targetBranch}`,
      setUpstream: isNew,
      forceWithLease: force,
    });
  };

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onCancel();
      }}
    >
      <Dialog.Content maxWidth={DIALOG_WIDTH}>
        <Dialog.Title>Push Commits</Dialog.Title>
        <Dialog.Description>
          Push {branch} to {targetRemote}/{targetBranch}
        </Dialog.Description>
        <form className={styles.dialogForm} id={formId} onSubmit={submit}>
          <div className={styles.pushTarget}>
            <code>{branch}</code>
            <span aria-hidden="true">→</span>
            {isNew ? (
              <span className={styles.remotePicker}>
                <FieldSelect
                  aria-label="Remote"
                  onChange={setRemote}
                  options={remoteOptions}
                  value={remote}
                />
              </span>
            ) : null}
            <code>
              {isNew ? "" : `${targetRemote}/`}
              {targetBranch}
            </code>
            {isNew ? (
              <Badge size="xs" tone="success">
                New
              </Badge>
            ) : null}
          </div>
          {isNew && upstreamMismatch ? (
            <p className={styles.mutedText}>
              {branch} tracks {upstream ?? "an upstream"} under a different
              name. Pushing to{" "}
              <code>
                {remote}/{branch}
              </code>{" "}
              sets it as the new upstream.
            </p>
          ) : isNew ? (
            <p className={styles.mutedText}>
              The branch has no upstream yet. Pushing creates{" "}
              <code>
                {remote}/{branch}
              </code>{" "}
              and sets it as upstream.
            </p>
          ) : (
            <OutgoingCommits count={ahead ?? 0} root={root} />
          )}
          <Checkbox
            checked={force}
            className={styles.dangerCheck}
            onCheckedChange={(checked) => setForce(checked === true)}
          >
            <span className={styles.dangerText}>Force with lease</span>
          </Checkbox>
        </form>
        {error ? (
          <div className={styles.dialogError} role="alert">
            <GitErrorDetails error={error} />
          </div>
        ) : null}
        <Dialog.Footer>
          <Button disabled={busy} onClick={onCancel} variant="soft">
            Cancel
          </Button>
          <Button
            form={formId}
            loading={busy}
            type="submit"
            variant={force ? "danger" : "primary"}
          >
            {force ? "Force Push" : "Push"}
          </Button>
        </Dialog.Footer>
      </Dialog.Content>
    </Dialog>
  );
}

/* eslint-disable react-refresh/only-export-components */
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";
import { ExternalLink, Send } from "lucide-react";

import { Button, Icon } from "../../components/ui";
import {
  reviewErrorMessage,
  useSendReviewItemToPrMutation,
  useSyncReviewCommentsMutation,
  type RemoteLink,
  type Review,
  type ReviewSyncReport,
} from "../../services/refact/review";
import { useReviewReadOnly } from "./readOnly";
import { remoteLink, setPublishComments, usePublishComments } from "./remote";
import styles from "./ReviewRemote.module.css";

const SYNC_INTERVAL_MS = 60_000;

export type ReviewRemote = {
  /** `owner/name#12` once the review is known to belong to a pull request. */
  pullRequest: string | null;
  /** The engine reaches the pull request itself; otherwise the chat is asked to. */
  available: boolean;
  reason: string | null;
  syncing: boolean;
  syncedAt: number | null;
  failed: NonNullable<ReviewSyncReport["failed"]>;
  syncNow: () => void;
  publish: boolean;
  setPublish: (value: boolean) => void;
};

export const ReviewRemoteContext = createContext<ReviewRemote | null>(null);

/** Keeps the pull request's discussion in step with the canvas while it is open. */
export function useReviewRemote(reviewId: string, active: boolean) {
  const [sync, { isLoading }] = useSyncReviewCommentsMutation();
  const readOnly = useReviewReadOnly();
  const publish = usePublishComments();
  const [report, setReport] = useState<ReviewSyncReport | null>(null);
  const [syncedAt, setSyncedAt] = useState<number | null>(null);
  const run = useCallback(
    async (quiet: boolean) => {
      const result = await sync({ reviewId, quiet });
      if ("data" in result && result.data) {
        setReport(result.data);
        setSyncedAt(Date.now());
      }
    },
    [sync, reviewId],
  );
  useEffect(() => {
    if (!active || readOnly) return;
    void run(true);
    const timer = window.setInterval(() => {
      if (!document.hidden) void run(true);
    }, SYNC_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [active, readOnly, run]);
  return useMemo<ReviewRemote>(
    () => ({
      pullRequest: report?.pull_request ?? null,
      available: report?.available ?? false,
      reason: report?.reason ?? null,
      syncing: isLoading,
      syncedAt,
      failed: report?.failed ?? [],
      syncNow: () => void run(false),
      publish,
      setPublish: setPublishComments,
    }),
    [report, isLoading, syncedAt, run, publish],
  );
}

/** True when a comment written now should also be posted to the pull request. */
export function usePublishToPr(): boolean {
  const remote = useContext(ReviewRemoteContext);
  return !!remote?.pullRequest && remote.publish;
}

/** "Also post to the PR" choice shown under a comment draft. */
export function PublishChoice() {
  const remote = useContext(ReviewRemoteContext);
  if (!remote?.pullRequest) return null;
  return (
    <label className={styles.choice}>
      <input
        type="checkbox"
        checked={remote.publish}
        onChange={(event) => remote.setPublish(event.target.checked)}
      />
      Also post to {remote.pullRequest}
    </label>
  );
}

function linkLabel(link: RemoteLink): string {
  if (link.state === "imported") return "From the PR";
  return link.state === "published" ? "On the PR" : "Waiting to send";
}

/** Where one finding or comment stands with the pull request, and the way to send it. */
export function PrState({
  review,
  kind,
  localId,
  sendable = true,
}: {
  review: Review;
  kind: RemoteLink["kind"];
  localId: string;
  /** Other people's comments are never ours to send. */
  sendable?: boolean;
}) {
  const remote = useContext(ReviewRemoteContext);
  const readOnly = useReviewReadOnly();
  const [send, state] = useSendReviewItemToPrMutation();
  const link = remoteLink(review, kind, localId);
  if (!link && (!remote?.pullRequest || !sendable)) return null;
  const failure = remote?.failed.find((item) => item.id === localId);
  if (link?.state === "published" || link?.state === "imported") {
    return link.url ? (
      <a
        className={styles.state}
        href={link.url}
        target="_blank"
        rel="noreferrer"
      >
        {linkLabel(link)} <Icon icon={ExternalLink} size="sm" />
      </a>
    ) : (
      <span className={styles.state}>{linkLabel(link)}</span>
    );
  }
  return (
    <span className={styles.state}>
      {link && (
        <span title={failure?.reason}>
          {failure ? `Not sent: ${failure.reason}` : linkLabel(link)}
        </span>
      )}
      {state.error !== undefined && (
        <span role="alert">{reviewErrorMessage(state.error)}</span>
      )}
      <Button
        type="button"
        size="sm"
        variant="ghost"
        loading={state.isLoading}
        disabled={readOnly}
        onClick={() => void send({ reviewId: review.id, id: localId })}
      >
        <Icon icon={Send} size="sm" /> {link ? "Send again" : "Send to PR"}
      </Button>
    </span>
  );
}

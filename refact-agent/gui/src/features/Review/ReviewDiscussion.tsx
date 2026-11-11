import { useContext } from "react";
import { RefreshCw } from "lucide-react";

import { Button, Icon } from "../../components/ui";
import type { Review, ReviewThread } from "../../services/refact/review";
import { usePendingDeliveries } from "./diff/usePendingDeliveries";
import { formatTime } from "./labels";
import { ThreadCard } from "./ReviewObjects";
import { ReviewRemoteContext } from "./ReviewRemote";
import styles from "./ReviewRemote.module.css";

function lastActivity(thread: ReviewThread): number {
  return Math.max(0, ...thread.contributions.map((item) => item.created_at_ms));
}

/** Every discussion of the review: the pull request's comments and the local ones. */
export function ReviewDiscussion({
  review,
  activeObjectId,
  onShow,
}: {
  review: Review;
  activeObjectId: string | null;
  onShow: (thread: ReviewThread) => void;
}) {
  const remote = useContext(ReviewRemoteContext);
  const pending = usePendingDeliveries(
    review.id,
    review.threads.some((thread) =>
      thread.contributions.some((item) => item.actor.kind === "user"),
    ),
  );
  const threads = [...review.threads].sort(
    (a, b) =>
      Number(a.resolved) - Number(b.resolved) ||
      lastActivity(b) - lastActivity(a),
  );
  return (
    <div className={styles.pane}>
      {remote?.pullRequest && (
        <div className={styles.bar}>
          <span className={styles.barIdentity}>
            Pull request {remote.pullRequest}
          </span>
          <span role="status">
            {remote.available
              ? remote.failed.length > 0
                ? `${remote.failed.length} not sent`
                : remote.syncedAt
                  ? `In sync · ${formatTime(remote.syncedAt)}`
                  : "In sync"
              : `Automatic sync is off: ${remote.reason ?? "unavailable"}`}
          </span>
          <label className={styles.choice}>
            <input
              type="checkbox"
              checked={remote.publish}
              onChange={(event) => remote.setPublish(event.target.checked)}
            />
            Post my comments to the PR
          </label>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            loading={remote.syncing}
            onClick={remote.syncNow}
          >
            <Icon icon={RefreshCw} size="sm" /> Sync now
          </Button>
        </div>
      )}
      <div className={styles.list}>
        {threads.length === 0 && (
          <p className={styles.empty}>
            No discussion yet. Comment on a line under Changes, or ask about a
            block on a tour page.
          </p>
        )}
        {threads.map((thread) => (
          <ThreadCard
            key={thread.id}
            review={review}
            thread={thread}
            active={thread.id === activeObjectId}
            pending={pending}
            location
            onFocus={() => onShow(thread)}
          />
        ))}
      </div>
    </div>
  );
}

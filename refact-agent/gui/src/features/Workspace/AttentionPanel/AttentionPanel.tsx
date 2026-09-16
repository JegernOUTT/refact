import { Inbox, RefreshCw } from "lucide-react";
import {
  Badge,
  Button,
  EmptyState,
  ErrorState,
  IconButton,
  LoadingState,
} from "../../../components/ui";
import { useConfig } from "../../../hooks/useConfig";
import { getEngineEndpointIdentity } from "../../../services/refact/apiUrl";
import { useGetAttentionQueueQuery } from "../../../services/refact/attention";
import {
  isSaturated,
  silentDefaults,
  type AttentionEntry,
  type AttentionQueue,
} from "../../../services/refact/attentionTypes";
import { humanizeIdentifier } from "../../../utils/displayNames";
import styles from "./AttentionPanel.module.css";

function MergedWaiters({ entry }: { entry: AttentionEntry }) {
  if (entry.merged_waiters.length === 0) return null;
  return (
    <ul
      className={styles.merged}
      aria-label={`Deduped into ${entry.question_id}`}
    >
      <li>Asked by {entry.asked_by}</li>
      {entry.merged_waiters.map((waiter) => (
        <li key={waiter}>{waiter} was merged into this same question</li>
      ))}
    </ul>
  );
}

function WaitingItem({ entry }: { entry: AttentionEntry }) {
  return (
    <li className={styles.item}>
      <div className={styles.heading}>
        <strong className="rf-wrap-anywhere">Rank {entry.rank}</strong>
        <Badge tone={entry.priority === "blocking" ? "warning" : "muted"}>
          {humanizeIdentifier(entry.priority)}
        </Badge>
      </div>
      <p className={styles.text}>{entry.text}</p>
      <span className={styles.muted} aria-label={`Rank reason ${entry.rank}`}>
        {entry.rank_reason}
      </span>
      <MergedWaiters entry={entry} />
      {entry.default_answer !== undefined && (
        <span className={styles.muted}>
          Defaults to &quot;{entry.default_answer}&quot; if nobody answers.
        </span>
      )}
    </li>
  );
}

function SettledItem({ entry }: { entry: AttentionEntry }) {
  const defaulted = entry.state === "defaulted";
  return (
    <li className={styles.item}>
      <div className={styles.heading}>
        <strong className="rf-wrap-anywhere">
          {humanizeIdentifier(entry.state)}
        </strong>
        <Badge tone={defaulted ? "warning" : "muted"}>
          {defaulted ? "Answered by timeout" : humanizeIdentifier(entry.state)}
        </Badge>
      </div>
      <p className={styles.text}>{entry.text}</p>
      {defaulted && (
        <p className={styles.warning} role="status">
          Nobody answered in time. The documented default &quot;
          {entry.default_used}&quot; was applied without your consent.
        </p>
      )}
      <MergedWaiters entry={entry} />
    </li>
  );
}

function SilentDefaultNotice({ queue }: { queue: AttentionQueue }) {
  const defaults = silentDefaults(queue);
  if (defaults.length === 0) return null;
  return (
    <p role="status" aria-label="Silent defaults" className={styles.warning}>
      {defaults.length} question{defaults.length === 1 ? "" : "s"} timed out
      into a default instead of being answered.
    </p>
  );
}

export function AttentionPanel() {
  const config = useConfig();
  const endpoint = getEngineEndpointIdentity(config);
  return <AttentionContents key={endpoint} endpoint={endpoint} />;
}

function AttentionContents({ endpoint }: { endpoint: string }) {
  const {
    currentData: queue,
    isFetching,
    error,
    refetch,
  } = useGetAttentionQueueQuery({ endpoint });
  return (
    <section className={styles.panel} aria-label="Attention queue">
      <header className={styles.heading}>
        <h2>Attention queue</h2>
        <IconButton
          aria-label="Refresh attention queue"
          icon={RefreshCw}
          variant="ghost"
          size="sm"
          onClick={() => void refetch()}
          disabled={isFetching}
        />
      </header>
      <p className={styles.muted}>
        What is waiting on you, ranked by the work stalled behind it. This view
        does not answer anything.
      </p>
      {error ? (
        <ErrorState
          title="Attention queue unavailable"
          description="The snapshot is unavailable, denied or expired. Refresh to request a new snapshot."
          retry={<Button onClick={() => void refetch()}>Refresh</Button>}
        />
      ) : !queue ? (
        <LoadingState label="Loading attention queue" />
      ) : (
        <>
          <p role="status" className={styles.muted}>
            {queue.outstanding} waiting of {queue.cap} allowed
            {isSaturated(queue)
              ? " · saturated, new low-priority questions are refused"
              : ""}
          </p>
          <SilentDefaultNotice queue={queue} />
          <div className={styles.body}>
            {queue.waiting.length === 0 ? (
              <EmptyState
                icon={Inbox}
                title="Nothing is waiting on you"
                description="No question is currently queued for your attention."
                variant="compact"
              />
            ) : (
              <ul className={styles.list} aria-label="Waiting questions">
                {queue.waiting.map((entry) => (
                  <WaitingItem key={entry.question_id} entry={entry} />
                ))}
              </ul>
            )}
            {queue.settled.length > 0 && (
              <section aria-label="Settled without you">
                <div className={styles.heading}>
                  <strong>Settled</strong>
                  <Badge tone={queue.silent_defaults > 0 ? "warning" : "muted"}>
                    {queue.silent_defaults} defaulted
                  </Badge>
                </div>
                <ul className={styles.list}>
                  {queue.settled.map((entry) => (
                    <SettledItem key={entry.question_id} entry={entry} />
                  ))}
                </ul>
              </section>
            )}
          </div>
        </>
      )}
    </section>
  );
}

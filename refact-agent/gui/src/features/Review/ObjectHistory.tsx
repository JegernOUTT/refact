import { useState } from "react";
import { History } from "lucide-react";

import { Badge, Button, Popover, ToolCard } from "../../components/ui";
import { Markdown } from "../../components/Markdown";
import {
  reviewErrorMessage,
  useGetReviewHistoryQuery,
  type Anchor,
  type HistoricalObject,
} from "../../services/refact/review";
import { DISPOSITION_TONE, formatTime, label } from "./labels";
import { priorRevisions, shortOid } from "./model";
import styles from "./Review.module.css";

const MAX_REVISIONS = 20;

function anchorsOf(object: HistoricalObject): Anchor[] {
  if (object.kind === "tour_step") return object.value.current.anchors;
  return object.value.anchors;
}

function RevisionBody({ object }: { object: HistoricalObject }) {
  if (object.kind === "annotation") {
    const a = object.value;
    return (
      <>
        <div className={styles.objectHead}>
          <strong className={styles.grow}>{a.title}</strong>
          <Badge tone={DISPOSITION_TONE[a.disposition]} size="xs">
            {label(a.disposition)}
          </Badge>
          <Badge tone="default" size="xs">
            {label(a.assessment)}
          </Badge>
        </div>
        <Markdown>{a.body}</Markdown>
      </>
    );
  }
  if (object.kind === "tour_step") {
    const v = object.value.current;
    return (
      <>
        <strong>
          v{v.version} · {v.title}
        </strong>
        <Markdown>{v.body}</Markdown>
      </>
    );
  }
  const t = object.value;
  return (
    <span className={styles.meta}>
      {t.contributions.length} contribution(s)
      {t.resolved ? " · resolved" : ""}
    </span>
  );
}

function revisionCandidate(object: HistoricalObject): string | null {
  const anchor = anchorsOf(object).at(0);
  return anchor ? anchor.position.candidate_id : null;
}

/**
 * Prior stored versions of an annotation, thread or tour step. History is
 * fetched only when opened and shown bounded, newest first.
 */
export function ObjectHistory({
  reviewId,
  current,
  compact = false,
}: {
  reviewId: string;
  current: HistoricalObject;
  compact?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const { data, error, isFetching } = useGetReviewHistoryQuery(
    { reviewId },
    { skip: !open },
  );
  const prior = data ? priorRevisions(data, current) : [];
  const shown = prior.slice(0, MAX_REVISIONS);

  const body = (
    <>
      {open ? (
        <div className={styles.history}>
          {isFetching && <span className={styles.meta}>Loading history…</span>}
          {error && (
            <p className={styles.errorText}>{reviewErrorMessage(error)}</p>
          )}
          {data && prior.length === 0 && (
            <span className={styles.meta}>No earlier versions.</span>
          )}
          {shown.map((r) => {
            const candidateId = revisionCandidate(r.object);
            return (
              <div
                key={r.revision}
                className={styles.contribution}
                aria-label={`Revision ${r.revision}`}
              >
                <span className={styles.meta}>
                  Revision {r.revision} · {label(r.actor.kind)} ·{" "}
                  {formatTime(r.at_ms)}
                  {candidateId ? ` · candidate ${shortOid(candidateId)}` : ""}
                </span>
                <RevisionBody object={r.object} />
              </div>
            );
          })}
          {prior.length > shown.length && (
            <span className={styles.meta}>
              Showing {shown.length} of {prior.length} earlier versions.
            </span>
          )}
        </div>
      ) : (
        <div />
      )}
    </>
  );
  return compact ? (
    <Popover open={open} onOpenChange={setOpen}>
      <Popover.Trigger asChild>
        <Button
          size="sm"
          variant="ghost"
          leftIcon={History}
          aria-label="Topic history"
        >
          History
        </Button>
      </Popover.Trigger>
      <Popover.Content align="end">{body}</Popover.Content>
    </Popover>
  ) : (
    <ToolCard
      title="History"
      icon={History}
      density="compact"
      open={open}
      onOpenChange={setOpen}
    >
      {body}
    </ToolCard>
  );
}

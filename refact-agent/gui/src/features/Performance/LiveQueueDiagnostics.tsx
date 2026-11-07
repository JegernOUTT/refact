import { Badge, Surface } from "../../components/ui";
import type {
  LiveQueueDiagnostics as Diagnostics,
  PendingAggregate,
} from "../../services/refact/performance";
import { humanizeIdentifier } from "../../utils/displayNames";
import {
  formatCount,
  formatDurationUs,
  formatTimestamp,
} from "./performanceFormatters";
import styles from "./LiveQueueDiagnostics.module.css";

function age(value: number | null): string {
  return value === null || !Number.isFinite(value)
    ? "Unknown"
    : formatDurationUs(value * 1_000);
}

function Aggregate({
  label,
  value,
}: {
  label: string;
  value: PendingAggregate;
}) {
  return (
    <div className={styles.stat}>
      <dt>{label}</dt>
      <dd>{formatCount(value.pending_count)}</dd>
      <dd>
        Oldest age:{" "}
        {value.pending_count === 0
          ? "No pending items"
          : age(value.oldest_pending_age_ms)}
      </dd>
      <dd>Unknown ages: {formatCount(value.unknown_age_count)}</dd>
    </div>
  );
}

const phaseLabels: Record<string, string | undefined> = {
  accepted: "Accepted / awaiting preparation",
  preamble: "Preamble",
  request_preparing: "Request preparation",
  stream_invocation: "Stream invocation",
};

export function LiveQueueDiagnostics({
  diagnostics,
}: {
  diagnostics?: Diagnostics | null;
}) {
  if (!diagnostics) {
    return (
      <Surface className={styles.panel} variant="glass">
        <h2>Live queue diagnostics</h2>
        <p>Live queue diagnostics unavailable.</p>
      </Surface>
    );
  }

  const { coverage, progress } = diagnostics;
  return (
    <Surface className={styles.panel} variant="glass">
      <div className={styles.heading}>
        <div>
          <h2>Live queue diagnostics</h2>
          <p>Live aggregate snapshot, independent of historical collection.</p>
        </div>
        <Badge tone={coverage.partial ? "warning" : "muted"} variant="soft">
          {coverage.partial ? "Partial coverage" : "Snapshot"}
        </Badge>
      </div>
      <p>
        Sampled {formatTimestamp(diagnostics.sampled_at_ms)} · Observed{" "}
        {formatCount(coverage.inspected)} of{" "}
        {coverage.sessions_total === null
          ? "unknown"
          : formatCount(coverage.sessions_total)}{" "}
        sessions
      </p>
      {coverage.partial && (
        <p role="status">
          Partial coverage: {formatCount(coverage.skipped_busy)} sessions
          skipped because they were busy; {formatCount(coverage.skipped_budget)}{" "}
          partially sampled or skipped by the sampling limit. Counts are lower
          bounds.
        </p>
      )}
      <h3>Accepted messages awaiting first output</h3>
      <p>
        Messages, including question replies, are in the transcript, but first
        output is pending. This includes preamble and request preparation;
        provider admission can occur during preparation. Stream invocation is
        not pure provider latency.
      </p>
      <dl className={styles.stats}>
        <Aggregate
          label="Awaiting first output"
          value={diagnostics.accepted_waiting_first_output}
        />
        {diagnostics.phases.map((entry) => (
          <Aggregate
            key={entry.phase}
            label={phaseLabels[entry.phase] ?? humanizeIdentifier(entry.phase)}
            value={entry}
          />
        ))}
      </dl>
      <h3>Pending work</h3>
      <p>
        Question answers are included in pending commands, not an additional
        total.
      </p>
      <dl className={styles.stats}>
        <Aggregate label="Commands" value={diagnostics.commands} />
        <Aggregate label="Deliveries" value={diagnostics.deliveries} />
        <Aggregate
          label="Question answers"
          value={diagnostics.question_answers}
        />
      </dl>
      {diagnostics.blockers.length > 0 && (
        <>
          <h3>Pending by reason</h3>
          <p>
            Observed queue gates; delivery boundaries are not proof of the root
            cause.
          </p>
          <dl className={styles.stats}>
            {diagnostics.blockers.map((entry) => (
              <Aggregate
                key={entry.reason}
                label={humanizeIdentifier(entry.reason)}
                value={entry}
              />
            ))}
          </dl>
        </>
      )}
      <h3>Progress ages</h3>
      <dl className={styles.stats}>
        <div className={styles.stat}>
          <dt>Oldest queue progress</dt>
          <dd>{age(progress.oldest_queue_progress_age_ms)}</dd>
          <dd>
            Unknown sessions:{" "}
            {formatCount(progress.unknown_queue_progress_sessions)}
          </dd>
        </div>
        <div className={styles.stat}>
          <dt>Oldest chat progress</dt>
          <dd>{age(progress.oldest_chat_progress_age_ms)}</dd>
          <dd>
            Unknown sessions:{" "}
            {formatCount(progress.unknown_chat_progress_sessions)}
          </dd>
        </div>
      </dl>
    </Surface>
  );
}

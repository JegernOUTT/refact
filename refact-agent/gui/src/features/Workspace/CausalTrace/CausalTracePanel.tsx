import { useEffect, useState } from "react";
import { GitBranch, RefreshCw } from "lucide-react";
import {
  Badge,
  Button,
  EmptyState,
  ErrorState,
  IconButton,
  LoadingState,
} from "../../../components/ui";
import {
  causalApi,
  useGetCausalTraceQuery,
} from "../../../services/refact/causal";
import {
  causeIds,
  hasUnknownCause,
  missingCauses,
  type CausalEvent,
  type CausalPage,
  type ReplayDivergence,
  type ReplayReport,
} from "../../../services/refact/causalTypes";
import { humanizeIdentifier } from "../../../utils/displayNames";
import { isExpiredCursor, panelErrorDescription } from "../panelError";
import { useAppDispatch } from "../../../hooks";
import { usePanelScope } from "../usePanelScope";
import styles from "./CausalTracePanel.module.css";
import { useCausalStream } from "./useCausalStream";

function describeDivergence(divergence: ReplayDivergence): string {
  if (divergence.kind === "cause_not_yet_admitted")
    return `Cause ${
      divergence.cause ?? "unnamed"
    } had not been admitted when this step ran.`;
  if (divergence.kind === "admission_order")
    return `Admission order diverged: recorded ${String(
      divergence.recorded,
    )}, replayed ${String(divergence.replayed)}.`;
  return `Completeness verdict diverged: recorded ${String(
    divergence.recorded,
  )}, replayed ${String(divergence.replayed)}.`;
}

function ReplaySection({ replay }: { replay: ReplayReport }) {
  return (
    <section aria-label="Replay of recorded decisions">
      <div className={styles.heading}>
        <strong>Replay</strong>
        <Badge tone={replay.finding_count > 0 ? "warning" : "muted"}>
          {replay.finding_count > 0
            ? `${replay.finding_count} divergence${
                replay.finding_count === 1 ? "" : "s"
              }`
            : "Reproduced"}
        </Badge>
      </div>
      <p className={styles.muted}>{replay.scope_note}</p>
      <p role="status" className={styles.muted}>
        {replay.finding_count === 0
          ? `${replay.steps} recorded coordination steps replayed identically.`
          : `${replay.steps} recorded coordination steps replayed; ${replay.finding_count} did not match what was recorded.`}
      </p>
      {replay.findings.length > 0 && (
        <ul className={styles.findings} aria-label="Replay divergences">
          {replay.findings.map((finding) => (
            <li
              className={styles.finding}
              key={`${finding.step}:${finding.divergence.kind}:${
                finding.divergence.cause ?? ""
              }`}
            >
              <strong className="rf-wrap-anywhere">{finding.step}</strong>{" "}
              <span className={styles.muted}>in {finding.scope}</span>
              <p className={styles.gap}>
                {describeDivergence(finding.divergence)}
              </p>
            </li>
          ))}
        </ul>
      )}
      {replay.finding_count > replay.findings.length && (
        <p className={styles.muted}>
          {replay.findings.length} of {replay.finding_count} divergences shown.
        </p>
      )}
    </section>
  );
}

function CompletenessNotice({ page }: { page: CausalPage }) {
  if (page.complete) return null;
  const reasons = [
    page.gap_count > 0
      ? `${page.gap_count} cause${
          page.gap_count === 1 ? "" : "s"
        } could not be resolved`
      : null,
    page.durable ? null : "the durable ledger could not be read",
  ].filter((reason): reason is string => reason !== null);
  return (
    <p role="status" aria-label="Trace completeness" className={styles.gap}>
      This trace is incomplete
      {reasons.length > 0 ? `: ${reasons.join(", ")}` : ""}. What is missing is
      shown as a gap, not bridged.
    </p>
  );
}

function TraceEvent({
  event,
  page,
  onNavigate,
}: {
  event: CausalEvent;
  page: CausalPage;
  onNavigate?: (id: string) => void;
}) {
  const missing = missingCauses(page, event);
  const resolved = causeIds(event).filter((id) => !missing.includes(id));
  return (
    <li className={styles.item}>
      <div className={styles.heading}>
        <strong className="rf-wrap-anywhere">{event.event_id}</strong>
        <Badge tone="muted">{humanizeIdentifier(event.kind)}</Badge>
      </div>
      <span className={styles.muted}>
        {event.scope} · {humanizeIdentifier(event.work.runtime)} · step{" "}
        {event.seq}
      </span>
      <ul className={styles.edges} aria-label={`Causes of ${event.event_id}`}>
        {resolved.map((id) => (
          <li className={styles.edge} key={id}>
            caused by {id}
          </li>
        ))}
        {missing.map((id) => (
          <li className={styles.gap} key={id}>
            Gap: {id} is not in this trace
          </li>
        ))}
        {hasUnknownCause(event) && (
          <li className={styles.gap}>
            Gap: the producer did not record what caused this
          </li>
        )}
      </ul>
      {onNavigate && event.work.runtime === "trajectory" && (
        <Button
          size="sm"
          variant="ghost"
          onClick={() => onNavigate(event.work.id)}
        >
          Open trajectory
        </Button>
      )}
    </li>
  );
}

export function CausalTracePanel({
  chatId,
  onNavigate,
}: {
  chatId?: string;
  onNavigate?: (id: string) => void;
}) {
  const endpoint = usePanelScope();
  return (
    <CausalTraceContents
      key={`${endpoint}:${chatId ?? ""}`}
      endpoint={endpoint}
      chatId={chatId}
      onNavigate={onNavigate}
    />
  );
}

function CausalTraceContents({
  endpoint,
  chatId,
  onNavigate,
}: {
  endpoint: string;
  chatId?: string;
  onNavigate?: (id: string) => void;
}) {
  const dispatch = useAppDispatch();
  const [draft, setDraft] = useState("");
  const [root, setRoot] = useState("");
  const [cursor, setCursor] = useState<string>();
  const {
    currentData: page,
    isFetching,
    error,
    refetch,
    fulfilledTimeStamp,
  } = useGetCausalTraceQuery(
    { endpoint, root, chatId, cursor },
    { skip: root === "", refetchOnMountOrArgChange: true },
  );
  const {
    error: streamError,
    retry: retryStream,
    reconnecting,
    blocked,
  } = useCausalStream(
    { endpoint, root, chatId },
    root !== "" && cursor === undefined && page !== undefined && !error,
    fulfilledTimeStamp,
  );
  useEffect(
    () => () => {
      dispatch(
        causalApi.util.getRunningQueryThunk("getCausalTrace", {
          endpoint,
          root,
          chatId,
          cursor,
        }),
      )?.abort();
    },
    [dispatch, endpoint, root, chatId, cursor],
  );
  useEffect(() => {
    if (!error) return;
    dispatch(
      causalApi.util.updateQueryData(
        "getCausalTrace",
        { endpoint, root, chatId, cursor },
        (draft) => {
          draft.events = [];
          draft.gaps = [];
          draft.total = 0;
          draft.returned = 0;
          draft.gap_count = 0;
          draft.complete = false;
          draft.replay.findings = [];
          draft.replay.finding_count = 0;
          draft.replay.steps = 0;
          delete draft.next_cursor;
        },
      ),
    );
  }, [error, dispatch, endpoint, root, chatId, cursor]);
  const refresh = () => {
    retryStream();
    setCursor(undefined);
    if (cursor === undefined && root !== "") void refetch();
  };
  useEffect(() => {
    if (cursor && isExpiredCursor(error)) setCursor(undefined);
  }, [cursor, error]);
  const stale = streamError && !blocked && page !== undefined && !error;
  return (
    <section className={styles.panel} aria-label="Causal trace">
      <header className={styles.heading}>
        <h2>Causal trace</h2>
        <IconButton
          aria-label="Refresh causal trace"
          icon={RefreshCw}
          variant="ghost"
          size="sm"
          onClick={refresh}
          disabled={isFetching || root === ""}
        />
      </header>
      <p className={styles.muted}>
        What caused what, across agents and runtimes, as it was recorded.
      </p>
      <form
        className={styles.rootForm}
        onSubmit={(submit) => {
          submit.preventDefault();
          setCursor(undefined);
          setRoot(draft.trim());
        }}
      >
        <input
          aria-label="Root event id"
          className={styles.rootInput}
          placeholder="Event id, e.g. delivery:abc"
          value={draft}
          onChange={(change) => setDraft(change.target.value)}
        />
        <Button size="sm" type="submit" disabled={draft.trim() === ""}>
          Trace
        </Button>
      </form>
      {isFetching && page && !error && !blocked && !streamError && (
        <p role="status">
          Refreshing snapshot. Previously loaded data may be stale.
        </p>
      )}
      {stale && (
        <p role="status">
          Updates interrupted. Showing a stale snapshot.
          {reconnecting
            ? " Reconnecting for a fresh snapshot."
            : " Automatic retries stopped. Refresh to try again."}
        </p>
      )}
      {root === "" ? (
        <EmptyState
          icon={GitBranch}
          title="No event selected"
          description="Enter a recorded event id to reconstruct what caused it."
          variant="compact"
        />
      ) : error !== undefined || blocked || (streamError !== null && !page) ? (
        <ErrorState
          title="Trace unavailable"
          description={panelErrorDescription(
            blocked ? streamError : error ?? streamError,
          )}
          retry={<Button onClick={refresh}>Refresh</Button>}
        />
      ) : !page ? (
        <LoadingState label="Loading causal trace" />
      ) : (
        <>
          <p role="status" className={styles.muted}>
            {page.total} recorded step{page.total === 1 ? "" : "s"} ·{" "}
            {page.returned} shown
          </p>
          <CompletenessNotice page={page} />
          <ReplaySection replay={page.replay} />
          <div className={styles.body}>
            {page.events.length === 0 ? (
              <EmptyState
                icon={GitBranch}
                title="Nothing recorded for this event"
                description="No causal event with this id is still held. That is a gap in the record, not proof that nothing happened."
                variant="compact"
              />
            ) : (
              <ul className={styles.list}>
                {page.events.map((event) => (
                  <TraceEvent
                    key={event.event_id}
                    event={event}
                    page={page}
                    onNavigate={onNavigate}
                  />
                ))}
              </ul>
            )}
          </div>
          <footer className={styles.heading}>
            <Button
              size="sm"
              variant="ghost"
              onClick={refresh}
              disabled={!cursor}
            >
              First page
            </Button>
            <Button
              size="sm"
              variant="ghost"
              disabled={!page.next_cursor || isFetching}
              onClick={() => setCursor(page.next_cursor)}
            >
              Next page
            </Button>
          </footer>
        </>
      )}
    </section>
  );
}

import { useEffect, useState } from "react";
import { Activity, RefreshCw } from "lucide-react";
import {
  Badge,
  Button,
  EmptyState,
  ErrorState,
  IconButton,
  LoadingState,
} from "../../../components/ui";
import { workApi, useGetWorkQuery } from "../../../services/refact/work";
import { humanizeIdentifier } from "../../../utils/displayNames";
import {
  withheldRowCount,
  type WorkRow,
} from "../../../services/refact/workTypes";
import { isExpiredCursor, panelErrorDescription } from "../panelError";
import { useAppDispatch } from "../../../hooks";
import { usePanelScope } from "../usePanelScope";
import styles from "./WorkPanel.module.css";
import { useWorkStream } from "./useWorkStream";

function WorkItem({
  row,
  onNavigate,
}: {
  row: WorkRow;
  onNavigate?: (id: string) => void;
}) {
  const filesByKind: [string, string[]][] = [
    ["Declared", row.declared_files],
    ["Read", row.observed_reads],
    ["Written", row.observed_files],
  ];
  // A withheld row deliberately omits waiting questions, admission class and
  // file activity: the privacy guard already reduced them, so rendering
  // "unknown" here would read as missing data rather than a policy decision.
  const withheld = row.disclosure === "withheld";
  return (
    <li className={styles.item}>
      <div className={styles.heading}>
        <strong className="rf-wrap-anywhere">
          {withheld
            ? "Work withheld by privacy policy"
            : row.title ?? humanizeIdentifier(row.kind)}
        </strong>
        <Badge
          tone={row.attention ? "warning" : "muted"}
          data-value={row.state}
          aria-label="Work state"
        >
          {humanizeIdentifier(row.state)}
        </Badge>
        {withheld && (
          <Badge tone="muted" aria-label="Withheld by privacy policy">
            Withheld
          </Badge>
        )}
      </div>
      <span className={styles.muted}>
        {humanizeIdentifier(row.kind)} ·{" "}
        <span aria-label="Freshness" data-value={row.freshness}>
          {humanizeIdentifier(row.freshness)}
        </span>
      </span>
      {row.attention && <span role="status">Needs attention</span>}
      {withheld ? (
        <p className={styles.muted}>
          This work exists, but its title, files and location are not available
          to you.
        </p>
      ) : (
        <WorkItemDetail row={row} filesByKind={filesByKind} />
      )}
      {row.chat_id && onNavigate && (
        <Button
          size="sm"
          variant="ghost"
          onClick={() => onNavigate(row.chat_id ?? "")}
        >
          Open trajectory
        </Button>
      )}
    </li>
  );
}

function WaitingQuestions({ count }: { count?: number }) {
  return (
    <span aria-label="Waiting questions" data-value={count ?? "unknown"}>
      {count === undefined
        ? "Waiting questions: unknown"
        : count === 0
          ? "No waiting questions"
          : `${count} waiting ${count === 1 ? "question" : "questions"}`}
    </span>
  );
}

function AdmissionClass({ row }: { row: WorkRow }) {
  return (
    <span aria-label="Admission class" data-value={row.priority ?? "unknown"}>
      Admission class:{" "}
      {row.priority ? humanizeIdentifier(row.priority) : "unknown"}
    </span>
  );
}

function WorkItemDetail({
  row,
  filesByKind,
}: {
  row: WorkRow;
  filesByKind: [string, string[]][];
}) {
  return (
    <>
      {row.summary && <p className={styles.summary}>{row.summary}</p>}
      {row.current_tool && <span>Tool: {row.current_tool}</span>}
      <WaitingQuestions count={row.open_questions} />
      <AdmissionClass row={row} />
      <span className="rf-wrap-anywhere">
        {row.root.branch ?? row.root.path ?? "Location unavailable"}
      </span>
      <details>
        <summary>
          File activity · {row.declared_files.length} declared ·{" "}
          {row.observed_files.length} written
        </summary>
        <div className={styles.details}>
          <p>
            Declared intent is not a file lock. Read/write coverage is partial.
          </p>
          {filesByKind.map(([label, files]) => (
            <div key={label}>
              <strong>{label}</strong>
              <ul>
                {files.map((file) => (
                  <li className="rf-wrap-anywhere" key={file}>
                    {file}
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
      </details>
    </>
  );
}

function WithheldNotice({ count }: { count: number | null }) {
  if (count === null) return null;
  return (
    <p role="status" aria-label="Withheld work count" className={styles.muted}>
      {count} {count === 1 ? "row was" : "rows were"} reduced by your privacy
      policy.
    </p>
  );
}

export function WorkPanel({
  onNavigate,
}: {
  onNavigate?: (id: string) => void;
}) {
  const endpoint = usePanelScope();
  return (
    <WorkPanelContents
      key={endpoint}
      endpoint={endpoint}
      onNavigate={onNavigate}
    />
  );
}

function WorkPanelContents({
  endpoint,
  onNavigate,
}: {
  endpoint: string;
  onNavigate?: (id: string) => void;
}) {
  const dispatch = useAppDispatch();
  const [cursor, setCursor] = useState<string>();
  const {
    currentData: page,
    isFetching,
    error,
    refetch,
    fulfilledTimeStamp,
  } = useGetWorkQuery(
    { endpoint, cursor },
    { refetchOnMountOrArgChange: true },
  );
  const {
    error: streamError,
    retry: retryStream,
    reconnecting,
    blocked,
  } = useWorkStream(
    { endpoint },
    cursor === undefined && page !== undefined && !error,
    fulfilledTimeStamp,
  );
  useEffect(
    () => () => {
      dispatch(
        workApi.util.getRunningQueryThunk("getWork", { endpoint, cursor }),
      )?.abort();
    },
    [dispatch, endpoint, cursor],
  );
  useEffect(() => {
    if (!error) return;
    dispatch(
      workApi.util.updateQueryData("getWork", { endpoint, cursor }, (draft) => {
        draft.rows = [];
        draft.roots = [];
        draft.total = 0;
        draft.returned = 0;
        delete draft.withheld_rows;
        delete draft.next_cursor;
      }),
    );
  }, [error, dispatch, endpoint, cursor]);
  const refresh = () => {
    retryStream();
    setCursor(undefined);
    if (cursor === undefined) void refetch();
  };
  useEffect(() => {
    if (cursor && isExpiredCursor(error)) setCursor(undefined);
  }, [cursor, error]);
  const stale = streamError && !blocked && page !== undefined && !error;
  return (
    <section className={styles.panel} aria-label="Project work">
      <header className={styles.heading}>
        <h2>Project work</h2>
        <IconButton
          aria-label="Refresh project work"
          icon={RefreshCw}
          variant="ghost"
          size="sm"
          onClick={refresh}
          disabled={isFetching}
        />
      </header>
      <p className={styles.muted}>
        Chats and board agents in this engine. Discovery does not grant control
        authority.
      </p>
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
      {error !== undefined || blocked || (streamError !== null && !page) ? (
        <ErrorState
          title="Work unavailable"
          description={panelErrorDescription(
            blocked ? streamError : error ?? streamError,
          )}
          retry={<Button onClick={refresh}>Refresh</Button>}
        />
      ) : !page ? (
        <LoadingState label="Loading project work" />
      ) : (
        <>
          <p role="status" className={styles.muted}>
            {page.total} observed · {humanizeIdentifier(page.coverage)} coverage
          </p>
          <WithheldNotice count={withheldRowCount(page)} />
          <div className={styles.body}>
            {page.rows.length === 0 ? (
              <EmptyState
                icon={Activity}
                title="No observed work"
                description="No work was observed in this engine and scope. This does not mean files are safe to overwrite."
                variant="compact"
              />
            ) : (
              <ul className={styles.list}>
                {page.rows.map((row) => (
                  <WorkItem
                    row={row}
                    key={`${row.key.runtime}:${row.key.id}`}
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

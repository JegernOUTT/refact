import { useState } from "react";
import { Activity, RefreshCw } from "lucide-react";
import {
  Badge,
  Button,
  EmptyState,
  ErrorState,
  IconButton,
  LoadingState,
} from "../../../components/ui";
import { useConfig } from "../../../hooks/useConfig";
import { useGetWorkQuery } from "../../../services/refact/work";
import { getEngineEndpointIdentity } from "../../../services/refact/apiUrl";
import { humanizeIdentifier } from "../../../utils/displayNames";
import {
  withheldRowCount,
  type WorkRow,
} from "../../../services/refact/workTypes";
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
  const withheld = row.disclosure === "withheld";
  return (
    <li className={styles.item}>
      <div className={styles.heading}>
        <strong className="rf-wrap-anywhere">
          {withheld
            ? "Work withheld by privacy policy"
            : row.title ?? humanizeIdentifier(row.kind)}
        </strong>
        <Badge tone={row.attention ? "warning" : "muted"}>
          {humanizeIdentifier(row.state)}
        </Badge>
        {withheld && (
          <Badge tone="muted" aria-label="Withheld by privacy policy">
            Withheld
          </Badge>
        )}
      </div>
      <span className={styles.muted}>
        {humanizeIdentifier(row.kind)} · {humanizeIdentifier(row.freshness)}
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
  const config = useConfig();
  const endpoint = getEngineEndpointIdentity(config);
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
  const [cursor, setCursor] = useState<string>();
  const {
    currentData: page,
    isFetching,
    error,
    refetch,
  } = useGetWorkQuery({ endpoint, cursor });
  const { error: streamError, retry: retryStream } = useWorkStream(
    { endpoint },
    cursor === undefined && page !== undefined,
  );
  const refresh = () => {
    retryStream();
    setCursor(undefined);
    if (cursor === undefined) void refetch();
  };
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
      {streamError && (
        <p role="status">{streamError}. Reconnecting for a fresh snapshot.</p>
      )}
      {error ?? streamError ? (
        <ErrorState
          title="Work unavailable"
          description="The snapshot is unavailable, denied or expired. Refresh to request a new snapshot."
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

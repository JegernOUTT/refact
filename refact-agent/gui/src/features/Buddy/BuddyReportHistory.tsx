import { useState } from "react";
import { Button } from "../../components/ui";
import {
  useGetBuddyArtifactHistoryQuery,
  useGetBuddyWorkOutcomesQuery,
} from "../../services/refact/buddyOperator";
import type { ArtifactRevision, WorkItem } from "./operatorTypes";
import { humanizeIdentifier } from "../../utils/displayNames";
import { BuddyArtifactBody } from "./BuddyArtifactBody";
import { stripLeadingTitleHeading } from "./BuddyArtifactBodyText";
import { BuddyReportRun } from "./BuddyReportRun";
import {
  BuddyDisclosure,
  BuddyErrorCallout,
  BuddyLoading,
  BuddyRow,
  BuddyTimestamp,
} from "./ui";
import styles from "./BuddyReportHistory.module.css";

export function BuddyReportHistory({
  item,
  artifact,
  identity,
}: {
  item: WorkItem;
  artifact: ArtifactRevision;
  identity: string;
}) {
  const [cursor, setCursor] = useState<number>();
  const [open, setOpen] = useState(false);
  const [selectedRun, setSelectedRun] = useState<string>();
  const [runsOpen, setRunsOpen] = useState(false);
  const [runCursor, setRunCursor] = useState<number>();
  const runs = useGetBuddyWorkOutcomesQuery(
    {
      id: item.id,
      project_id: item.project.project_id,
      endpoint_identity: identity,
      cursor: runCursor,
    },
    { skip: !runsOpen },
  );
  const history = useGetBuddyArtifactHistoryQuery(
    {
      id: artifact.artifact_id,
      project_id: item.project.project_id,
      endpoint_identity: identity,
      cursor,
    },
    { skip: !open },
  );
  return (
    <div className={styles.history}>
      <BuddyDisclosure
        label="Earlier revisions"
        open={open}
        onOpenChange={setOpen}
        testId="buddy-report-revisions"
        contentClassName={styles.body}
      >
        {history.isFetching && (
          <BuddyLoading variant="rows" count={2} label="Loading revisions" />
        )}
        {history.error && (
          <BuddyErrorCallout
            thing="earlier revisions"
            error={history.error}
            onRetry={() => void history.refetch()}
            compact
          />
        )}
        {history.currentData?.data.map(({ value }) =>
          value.project.canonical_root !== item.project.canonical_root ? (
            <p key={value.revision} role="alert" className={styles.warning}>
              This revision belongs to another project.
            </p>
          ) : (
            <BuddyDisclosure
              key={value.revision}
              label={`Revision ${value.revision} · ${value.title}`}
              lazy
              contentClassName={styles.body}
            >
              <p className={styles.text}>{value.conclusion}</p>
              <BuddyArtifactBody
                body={stripLeadingTitleHeading(value.body, value.title)}
              />
              <p className={styles.hint}>
                Earlier revision. Actions apply to the current revision only.
              </p>
            </BuddyDisclosure>
          ),
        )}
        <div className={styles.pager}>
          {history.currentData?.next_cursor != null && (
            <Button
              size="sm"
              variant="soft"
              disabled={history.isFetching}
              onClick={() =>
                setCursor(history.currentData?.next_cursor ?? undefined)
              }
            >
              Older revisions
            </Button>
          )}
          {cursor !== undefined && (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setCursor(undefined)}
            >
              Newest revisions
            </Button>
          )}
        </div>
      </BuddyDisclosure>
      <BuddyDisclosure
        label="Run history"
        summary="Every check, including runs with no change"
        open={runsOpen}
        onOpenChange={setRunsOpen}
        testId="buddy-report-runs"
        contentClassName={styles.body}
      >
        {runs.isFetching && (
          <BuddyLoading variant="rows" count={2} label="Loading run history" />
        )}
        {runs.error && (
          <BuddyErrorCallout
            thing="run history"
            error={runs.error}
            onRetry={() => void runs.refetch()}
            compact
          />
        )}
        {!runs.error && (
          <ul className={styles.rows} aria-label="Runs">
            {runs.currentData?.data.map(({ value }) =>
              value.project.canonical_root !== item.project.canonical_root ? (
                <li key={value.run_id} role="alert" className={styles.warning}>
                  This run belongs to another project.
                </li>
              ) : (
                <li key={value.run_id}>
                  <BuddyRow
                    density="compact"
                    title={humanizeIdentifier(value.status)}
                    meta={[
                      <BuddyTimestamp
                        key="time"
                        value={value.ended_at ?? value.started_at}
                      />,
                    ]}
                    selected={selectedRun === value.run_id}
                    onClick={() => setSelectedRun(value.run_id)}
                  />
                </li>
              ),
            )}
          </ul>
        )}
        <div className={styles.pager}>
          {runs.currentData?.next_cursor != null && (
            <Button
              size="sm"
              variant="soft"
              disabled={runs.isFetching}
              onClick={() =>
                setRunCursor(runs.currentData?.next_cursor ?? undefined)
              }
            >
              Older runs
            </Button>
          )}
          {runCursor !== undefined && (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setRunCursor(undefined)}
            >
              Newest runs
            </Button>
          )}
        </div>
      </BuddyDisclosure>
      {artifact.source_run_ids.length > 0 && (
        <BuddyDisclosure
          label="Source runs"
          count={artifact.source_run_ids.length}
          contentClassName={styles.body}
        >
          <ul className={styles.rows} aria-label="Source runs">
            {artifact.source_run_ids.map((id, index) => (
              <li key={id}>
                <BuddyRow
                  density="compact"
                  title={`Source run ${index + 1}`}
                  selected={selectedRun === id}
                  onClick={() => setSelectedRun(id)}
                />
              </li>
            ))}
          </ul>
        </BuddyDisclosure>
      )}
      {selectedRun && (
        <BuddyReportRun
          key={selectedRun}
          runId={selectedRun}
          item={item}
          identity={identity}
          onClose={() => setSelectedRun(undefined)}
        />
      )}
    </div>
  );
}

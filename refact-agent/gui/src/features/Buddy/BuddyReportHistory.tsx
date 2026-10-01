import { useState } from "react";
import { Button, Text } from "../../components/ui";
import {
  useGetBuddyArtifactHistoryQuery,
  useGetBuddyRunOutcomeQuery,
  useGetBuddyWorkOutcomesQuery,
} from "../../services/refact/buddyOperator";
import type { ArtifactRevision, WorkItem } from "./operatorTypes";
import { humanizeIdentifier } from "../../utils/displayNames";
import { formatReportTime, humanizeResourceReference } from "./buddyUtils";
import { BuddyArtifactBody } from "./BuddyArtifactBody";
import styles from "./BuddyReportHistory.module.css";

function RunEvidence({
  runId,
  item,
  identity,
}: {
  runId: string;
  item: WorkItem;
  identity: string;
}) {
  const query = useGetBuddyRunOutcomeQuery({
    id: runId,
    project_id: item.project.project_id,
    endpoint_identity: identity,
  });
  const run = query.currentData?.data.value;
  const matches =
    run?.work_id === item.id &&
    run.project.canonical_root === item.project.canonical_root;
  const startedAt = formatReportTime(run?.started_at);
  const endedAt = formatReportTime(run?.ended_at);
  return (
    <section aria-label="Recorded run" className={styles.run}>
      {query.isFetching && (
        <Text as="p" size="1" color="gray" role="status">
          Loading recorded run…
        </Text>
      )}
      {Boolean(query.error) || (run && !matches) ? (
        <Text as="p" size="2" color="red" role="alert">
          Recorded run could not be matched to this work.
        </Text>
      ) : (
        run && (
          <>
            <Text
              as="p"
              size="1"
              color="gray"
              title={startedAt.absolute ?? undefined}
            >
              {humanizeIdentifier(run.status)} · {startedAt.relative} —{" "}
              {run.ended_at ? (
                <time title={endedAt.absolute ?? undefined}>
                  {endedAt.relative}
                </time>
              ) : (
                "No terminal time"
              )}
            </Text>
            <h3 className={styles.heading}>Observed scope</h3>
            <Text as="p" size="2">
              {run.coverage}
            </Text>
            {run.failure_class && (
              <Text as="p" size="1" color="orange">
                {humanizeIdentifier(run.failure_class)}
              </Text>
            )}
            {run.effects.length === 0 ? (
              <Text as="p" size="1" color="gray">
                No resource effects were recorded.
              </Text>
            ) : (
              <ul className={styles.effects}>
                {run.effects.map((effect, index) => (
                  <li key={index}>
                    <Text size="1" color="gray">
                      {humanizeResourceReference(
                        effect.resource.resource_kind,
                        effect.resource.resource_id,
                      )}
                      : {humanizeIdentifier(effect.state)}
                    </Text>
                  </li>
                ))}
              </ul>
            )}
          </>
        )
      )}
    </section>
  );
}

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
      <details
        className={styles.disclosure}
        onToggle={(event) => setOpen(event.currentTarget.open)}
      >
        <summary className={styles.summary}>Saved report revisions</summary>
        <div className={styles.body}>
          {history.isFetching && (
            <Text as="p" size="1" color="gray" role="status">
              Loading report history…
            </Text>
          )}
          {history.error && (
            <Text as="p" size="2" color="red" role="alert">
              Report history is unavailable.
            </Text>
          )}
          {history.currentData?.data.map(({ value }) =>
            value.project.canonical_root !== item.project.canonical_root ? (
              <Text
                as="p"
                size="2"
                color="red"
                key={value.revision}
                role="alert"
              >
                Historical report belongs to another project.
              </Text>
            ) : (
              <details key={value.revision} className={styles.disclosure}>
                <summary className={styles.summary}>
                  Revision {value.revision} · {value.title}
                </summary>
                <div className={styles.body}>
                  <Text as="p" size="2">
                    {value.conclusion}
                  </Text>
                  <BuddyArtifactBody body={value.body} />
                  <Text as="p" size="1" color="gray">
                    Historical evidence only. Actions are available on the
                    current revision.
                  </Text>
                </div>
              </details>
            ),
          )}
          {history.currentData?.next_cursor != null && (
            <Button
              size="sm"
              disabled={history.isFetching}
              onClick={() =>
                setCursor(history.currentData?.next_cursor ?? undefined)
              }
            >
              Next revisions
            </Button>
          )}
          {cursor !== undefined && (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setCursor(undefined)}
            >
              First revisions
            </Button>
          )}
        </div>
      </details>
      <details
        className={styles.disclosure}
        onToggle={(event) => setRunsOpen(event.currentTarget.open)}
      >
        <summary className={styles.summary}>
          All recorded checks, including unchanged runs
        </summary>
        <div className={styles.body}>
          <Text as="p" size="1" color="gray">
            Every repeat check is kept here, even when nothing changed.
          </Text>
          {runs.isFetching && (
            <Text as="p" size="1" color="gray" role="status">
              Loading recorded checks…
            </Text>
          )}
          {runs.error && (
            <Text as="p" size="2" color="red" role="alert">
              Recorded checks are unavailable.
            </Text>
          )}
          {!runs.error &&
            runs.currentData?.data.map(({ value }) =>
              value.project.canonical_root !== item.project.canonical_root ? (
                <Text
                  as="p"
                  size="2"
                  color="red"
                  role="alert"
                  key={value.run_id}
                >
                  Recorded check belongs to another project.
                </Text>
              ) : (
                <Button
                  key={value.run_id}
                  size="sm"
                  variant="ghost"
                  className={styles.runRow}
                  onClick={() => setSelectedRun(value.run_id)}
                  title={`Run ID: ${value.run_id} · ${new Date(
                    value.ended_at ?? value.started_at,
                  ).toLocaleString()}`}
                >
                  {humanizeIdentifier(value.status)} ·{" "}
                  <time
                    title={new Date(
                      value.ended_at ?? value.started_at,
                    ).toLocaleString()}
                  >
                    {
                      formatReportTime(value.ended_at ?? value.started_at)
                        .relative
                    }
                  </time>
                </Button>
              ),
            )}
          {runs.currentData?.next_cursor != null && (
            <Button
              size="sm"
              disabled={runs.isFetching}
              onClick={() =>
                setRunCursor(runs.currentData?.next_cursor ?? undefined)
              }
            >
              Next recorded checks
            </Button>
          )}
          {runCursor !== undefined && (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setRunCursor(undefined)}
            >
              First recorded checks
            </Button>
          )}
        </div>
      </details>
      {artifact.source_run_ids.length > 0 && (
        <details className={styles.disclosure}>
          <summary className={styles.summary}>Source runs</summary>
          <div className={styles.body}>
            {artifact.source_run_ids.map((id, index) => (
              <Button
                key={id}
                size="sm"
                variant="ghost"
                className={styles.runRow}
                onClick={() => setSelectedRun(id)}
                title={`Run ID: ${id}`}
              >
                Source run {index + 1}
              </Button>
            ))}
          </div>
        </details>
      )}
      {selectedRun && (
        <RunEvidence
          key={selectedRun}
          runId={selectedRun}
          item={item}
          identity={identity}
        />
      )}
    </div>
  );
}

import { useState } from "react";
import { Button } from "../../components/ui";
import {
  useGetBuddyArtifactHistoryQuery,
  useGetBuddyRunOutcomeQuery,
  useGetBuddyWorkOutcomesQuery,
} from "../../services/refact/buddyOperator";
import type { ArtifactRevision, WorkItem } from "./operatorTypes";
import { humanizeIdentifier } from "../../utils/displayNames";
import { BuddyArtifactBody } from "./BuddyArtifactBody";

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
  return (
    <section aria-label="Recorded run">
      {query.isFetching && <p role="status">Loading recorded run…</p>}
      {Boolean(query.error) || (run && !matches) ? (
        <p role="alert">Recorded run could not be matched to this work.</p>
      ) : (
        run && (
          <>
            <p>
              {humanizeIdentifier(run.status)} · {run.started_at} —{" "}
              {run.ended_at ?? "No terminal time"}
            </p>
            <h5>Observed scope</h5>
            <p>{run.coverage}</p>
            {run.failure_class && (
              <p>{humanizeIdentifier(run.failure_class)}</p>
            )}
            {run.effects.length === 0 ? (
              <p>No resource effects were recorded.</p>
            ) : (
              <ul>
                {run.effects.map((effect, index) => (
                  <li key={index}>
                    {effect.resource.resource_id}:{" "}
                    {humanizeIdentifier(effect.state)}
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
    <>
      <details onToggle={(event) => setOpen(event.currentTarget.open)}>
        <summary>Saved report revisions</summary>
        {history.isFetching && <p role="status">Loading report history…</p>}
        {history.error && <p role="alert">Report history is unavailable.</p>}
        {history.currentData?.data.map(({ value }) =>
          value.project.canonical_root !== item.project.canonical_root ? (
            <p key={value.revision} role="alert">
              Historical report belongs to another project.
            </p>
          ) : (
            <details key={value.revision}>
              <summary>
                Revision {value.revision} · {value.title}
              </summary>
              <p>{value.conclusion}</p>
              <BuddyArtifactBody body={value.body} />
              <p>
                Historical evidence only. Actions are available on the current
                revision.
              </p>
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
      </details>
      <details onToggle={(event) => setRunsOpen(event.currentTarget.open)}>
        <summary>All recorded checks, including unchanged runs</summary>
        <p>
          Repeated checks remain accountable here without creating a new unread
          report revision.
        </p>
        {runs.isFetching && <p role="status">Loading recorded checks…</p>}
        {runs.error && <p role="alert">Recorded checks are unavailable.</p>}
        {!runs.error &&
          runs.currentData?.data.map(({ value }) =>
            value.project.canonical_root !== item.project.canonical_root ? (
              <p role="alert" key={value.run_id}>
                Recorded check belongs to another project.
              </p>
            ) : (
              <Button
                key={value.run_id}
                size="sm"
                variant="ghost"
                onClick={() => setSelectedRun(value.run_id)}
              >
                {humanizeIdentifier(value.status)} ·{" "}
                {value.ended_at ?? value.started_at} · {value.run_id}
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
      </details>
      <details>
        <summary>Source runs</summary>
        {artifact.source_run_ids.map((id) => (
          <Button
            key={id}
            size="sm"
            variant="ghost"
            onClick={() => setSelectedRun(id)}
          >
            {id}
          </Button>
        ))}
      </details>
      {selectedRun && (
        <RunEvidence
          key={selectedRun}
          runId={selectedRun}
          item={item}
          identity={identity}
        />
      )}
    </>
  );
}

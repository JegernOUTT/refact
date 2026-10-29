import { X } from "lucide-react";
import { IconButton } from "../../components/ui";
import { useGetBuddyRunOutcomeQuery } from "../../services/refact/buddyOperator";
import type { WorkItem } from "./operatorTypes";
import { humanizeIdentifier } from "../../utils/displayNames";
import { humanizeResourceReference } from "./buddyUtils";
import {
  BUDDY_TERMS,
  BuddyDisclosure,
  BuddyEmptyState,
  BuddyErrorCallout,
  BuddyLoading,
  BuddySurface,
  buddyFailureLabel,
  formatBuddyRange,
} from "./ui";
import styles from "./BuddyReportHistory.module.css";

export function BuddyReportRun({
  runId,
  item,
  identity,
  onClose,
}: {
  runId: string;
  item: WorkItem;
  identity: string;
  onClose: () => void;
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
    <BuddySurface
      as="section"
      level="inset"
      aria-label="Recorded run"
      className={styles.run}
    >
      <div className={styles.runHeader}>
        <h4 className={styles.label}>
          {run && matches
            ? `${humanizeIdentifier(run.status)} · ${formatBuddyRange(
                run.started_at,
                run.ended_at,
              )}`
            : "Recorded run"}
        </h4>
        <IconButton
          icon={X}
          size="sm"
          variant="ghost"
          aria-label="Close recorded run"
          onClick={onClose}
        />
      </div>
      {query.isFetching && (
        <BuddyLoading variant="rows" count={1} label="Loading recorded run" />
      )}
      {Boolean(query.error) || (run && !matches) ? (
        <BuddyErrorCallout
          thing="this run"
          title="This run couldn't be matched to this report."
          error={query.error}
          compact
        />
      ) : (
        run && (
          <>
            {!run.ended_at && <p className={styles.hint}>Not finished yet.</p>}
            <h5 className={styles.label}>What was checked</h5>
            <p className={styles.text}>{run.coverage}</p>
            {run.failure_class && (
              <p className={styles.warning}>
                {buddyFailureLabel(run.failure_class)}
              </p>
            )}
            {run.effects.length === 0 ? (
              <BuddyEmptyState compact title="No changes were recorded." />
            ) : (
              <ul className={styles.effects}>
                {run.effects.map((effect, index) => (
                  <li key={index} className={styles.hint}>
                    {humanizeResourceReference(
                      effect.resource.resource_kind,
                      effect.resource.resource_id,
                    )}
                    : {humanizeIdentifier(effect.state)}
                  </li>
                ))}
              </ul>
            )}
            <BuddyDisclosure label={BUDDY_TERMS.technicalDetails} lazy>
              <pre className={styles.technical}>{`Run ${run.run_id}`}</pre>
            </BuddyDisclosure>
          </>
        )
      )}
    </BuddySurface>
  );
}

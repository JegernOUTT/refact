import { useMemo } from "react";
import { ScrollText } from "lucide-react";

import { Button, EmptyState, Surface } from "../../../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../../../hooks";
import type { DaemonWorker } from "../../../../../services/refact/daemon";
import { filterDaemonEvents } from "../../../Activity/activityState";
import { navigateDashboard, selectDaemonEvents } from "../../../dashboardSlice";
import styles from "../ProjectDetail.module.css";

const MAX_PROJECT_EVENTS = 30;

type ActivityTabProps = {
  worker: DaemonWorker;
};

function relativeEventTime(tsMs: number): string {
  const minutes = Math.max(0, Math.floor((Date.now() - tsMs) / 60_000));
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${String(minutes)}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${String(hours)}h ago`;
  return `${String(Math.floor(hours / 24))}d ago`;
}

export function ActivityTab({ worker }: ActivityTabProps) {
  const dispatch = useAppDispatch();
  const events = useAppSelector(selectDaemonEvents);
  const projectEvents = useMemo(
    () =>
      filterDaemonEvents(events, new Set(), worker.project_id)
        .slice(-MAX_PROJECT_EVENTS)
        .reverse(),
    [events, worker.project_id],
  );
  const openActivity = (
    <Button
      onClick={() =>
        dispatch(
          navigateDashboard({
            page: "activity",
            params: { projectId: worker.project_id },
          }),
        )
      }
      size="sm"
      variant="soft"
    >
      Open activity log
    </Button>
  );

  return (
    <div className={styles.tabBody}>
      {projectEvents.length === 0 ? (
        <EmptyState
          action={openActivity}
          description="Daemon events for this project will appear here."
          icon={ScrollText}
          title="No project events yet"
        />
      ) : (
        <Surface className={styles.section} radius="card" variant="glass">
          <h3 className={styles.sectionTitle}>Recent events</h3>
          <ul aria-label="Project events" className={styles.list}>
            {projectEvents.map((event) => (
              <li className={styles.row} key={event.seq}>
                <span className={styles.rowCopy}>
                  <strong>{event.kind}</strong>
                </span>
                <span className={styles.rowMeta}>
                  {relativeEventTime(event.ts_ms)}
                </span>
              </li>
            ))}
          </ul>
          <div className={styles.actions}>{openActivity}</div>
        </Surface>
      )}
    </div>
  );
}

import { useAppSelector } from "../../../hooks";
import { useListProjectsQuery } from "../../../services/refact/daemon";
import { selectDashboardParams } from "../dashboardSlice";
import styles from "./ActivityPage.module.css";
import { EventsTimeline } from "./EventsTimeline";
import { LogTailPane } from "./LogTailPane";

export function ActivityPage() {
  const { data: workers = [] } = useListProjectsQuery(undefined, {
    pollingInterval: 5_000,
  });
  const params = useAppSelector(selectDashboardParams);
  const projectIdParam = params.projectId;

  return (
    <section className={styles.page}>
      <header className={styles.pageHeader}>
        <div>
          <h1>Activity</h1>
          <p>Inspect daemon events and live process logs.</p>
        </div>
      </header>
      <div className={styles.panes}>
        <EventsTimeline projectIdParam={projectIdParam} workers={workers} />
        <LogTailPane projectIdParam={projectIdParam} workers={workers} />
      </div>
    </section>
  );
}

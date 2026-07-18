import { useListProjectsQuery } from "../../../services/refact/daemon";
import styles from "./ActivityPage.module.css";
import { EventsTimeline } from "./EventsTimeline";
import { LogTailPane } from "./LogTailPane";

export function ActivityPage() {
  const { data: workers = [] } = useListProjectsQuery(undefined, {
    pollingInterval: 5_000,
  });

  return (
    <section className={styles.page}>
      <header className={styles.pageHeader}>
        <div>
          <h1>Activity</h1>
          <p>Inspect daemon events and live process logs.</p>
        </div>
      </header>
      <div className={styles.panes}>
        <EventsTimeline workers={workers} />
        <LogTailPane workers={workers} />
      </div>
    </section>
  );
}

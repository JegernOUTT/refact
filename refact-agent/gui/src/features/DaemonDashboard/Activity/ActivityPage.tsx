import { useState } from "react";
import { Tabs } from "../../../components/ui";
import { useObservationPolling } from "../useObservationPolling";
import { useAppSelector } from "../../../hooks";
import { useListProjectsQuery } from "../../../services/refact/daemon";
import { selectDashboardParams } from "../dashboardSlice";
import styles from "./ActivityPage.module.css";
import { EventsTimeline } from "./EventsTimeline";
import { LogTailPane } from "./LogTailPane";

export function ActivityPage() {
  const { data: workers = [] } = useListProjectsQuery(
    undefined,
    useObservationPolling(),
  );
  const [pane, setPane] = useState("logs");
  const params = useAppSelector(selectDashboardParams);
  const projectIdParam = params.projectId;

  return (
    <section className={styles.page}>
      <header className={styles.pageHeader}>
        <div>
          <h1>Logs</h1>
          <p>Inspect daemon events and live process logs.</p>
        </div>
      </header>
      <Tabs
        value={pane}
        onValueChange={setPane}
        className={styles.activityTabs}
      >
        <Tabs.List activeIndex={pane === "logs" ? 0 : 1}>
          <Tabs.Trigger value="logs">Log tail</Tabs.Trigger>
          <Tabs.Trigger value="events">Events</Tabs.Trigger>
        </Tabs.List>
        <Tabs.Content value="logs" className={styles.panes}>
          <LogTailPane projectIdParam={projectIdParam} workers={workers} />
        </Tabs.Content>
        <Tabs.Content value="events" className={styles.panes}>
          <EventsTimeline projectIdParam={projectIdParam} workers={workers} />
        </Tabs.Content>
      </Tabs>
    </section>
  );
}

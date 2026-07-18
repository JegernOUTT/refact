import { Badge } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import { useDaemonEventsStream } from "../../hooks/useDaemonEventsStream";
import { useGetDaemonInfoQuery } from "../../services/refact/daemon";
import { ActivityPage } from "./Activity/ActivityPage";
import { DashboardNav } from "./DashboardNav";
import { DoctorPage } from "./Doctor/DoctorPage";
import { HomePage } from "./Home/HomePage";
import { ProjectDetailPage } from "./Projects/ProjectDetail/ProjectDetailPage";
import { ProjectsPage } from "./Projects/ProjectsPage";
import { SchedulerPlaceholderPage } from "./Scheduler/PlaceholderPage";
import { SettingsPage } from "./Settings/SettingsPage";
import { UsagePlaceholderPage } from "./Usage/PlaceholderPage";
import { selectDashboardNavigation } from "./dashboardSlice";
import styles from "./DaemonDashboard.module.css";

const DAEMON_POLLING_INTERVAL_MS = 5_000;

function formatUptime(totalSeconds: number): string {
  if (!Number.isFinite(totalSeconds) || totalSeconds < 60) {
    return `${Math.max(0, Math.floor(totalSeconds))}s`;
  }
  const hours = Math.floor(totalSeconds / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  return hours > 0 ? `${hours}h ${minutes}m` : `${minutes}m`;
}

function DashboardPageContent() {
  const navigation = useAppSelector(selectDashboardNavigation);
  const page = navigation.page;
  switch (page) {
    case "projects":
      return navigation.params.projectId ? (
        <ProjectDetailPage projectId={navigation.params.projectId} />
      ) : (
        <ProjectsPage />
      );
    case "activity":
      return <ActivityPage />;
    case "scheduler":
      return <SchedulerPlaceholderPage />;
    case "usage":
      return <UsagePlaceholderPage />;
    case "doctor":
      return <DoctorPage />;
    case "settings":
      return <SettingsPage />;
    case "home":
      return <HomePage />;
  }
}

export function DaemonDashboardShell() {
  const { data, error } = useGetDaemonInfoQuery(undefined, {
    pollingInterval: DAEMON_POLLING_INTERVAL_MS,
  });
  const status = data?.status;
  useDaemonEventsStream({ daemonStartedAtMs: status?.started_at_ms });
  const live = status !== undefined && error === undefined;

  return (
    <div className={styles.shell} data-testid="daemon-dashboard-shell">
      <aside className={styles.sidebar}>
        <div className={styles.brand}>
          <span className={styles.brandMark}>R</span>
          <span className={styles.brandText}>Refact</span>
        </div>
        <DashboardNav />
      </aside>
      <div className={styles.workspace}>
        <header className={styles.header}>
          <div>
            <span className={styles.eyebrow}>Daemon dashboard</span>
            <h1>Mission control</h1>
          </div>
          <div className={styles.status} aria-live="polite">
            <Badge tone={live ? "success" : "danger"} variant="soft">
              {live ? "Live" : "Unreachable"}
            </Badge>
            {status ? (
              <span>
                v{status.version} · {formatUptime(status.uptime_secs)} ·{" "}
                {status.workers} workers
              </span>
            ) : (
              <span>Check the daemon and reconnect.</span>
            )}
          </div>
        </header>
        <main className={styles.content}>
          <DashboardPageContent />
        </main>
      </div>
    </div>
  );
}

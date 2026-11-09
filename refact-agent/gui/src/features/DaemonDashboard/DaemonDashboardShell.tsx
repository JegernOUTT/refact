import { useState } from "react";
import { Menu } from "lucide-react";
import { humanizeIdentifier } from "../../utils/displayNames";
import { RefactIcon } from "../../images/RefactIcon";
import { EnginesPage } from "./EnginesPage";
import { ActiveChatsPage } from "./ActiveChatsPage";
import { QuickTour } from "./QuickTour";
import { useObservationPolling } from "./useObservationPolling";
import { Badge, Button, Sheet } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import { useDaemonEventsStream } from "../../hooks/useDaemonEventsStream";
import { useGetDaemonInfoQuery } from "../../services/refact/daemon";
import { ActivityPage } from "./Activity/ActivityPage";
import { DashboardNav } from "./DashboardNav";
import { DoctorPage } from "./Doctor/DoctorPage";
import { HomePage } from "./Home/HomePage";
import { ProjectDetailPage } from "./Projects/ProjectDetail/ProjectDetailPage";
import { ProjectsPage } from "./Projects/ProjectsPage";
import { SchedulerPage } from "./Scheduler/SchedulerPage";
import { SettingsPage } from "./Settings/SettingsPage";
import { UsagePage } from "./Usage/UsagePage";
import { selectDashboardNavigation } from "./dashboardSlice";
import styles from "./DaemonDashboard.module.css";

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
    case "engines":
      return <EnginesPage />;
    case "chats":
      return <ActiveChatsPage />;
    case "activity":
      return <ActivityPage />;
    case "scheduler":
      return <SchedulerPage />;
    case "usage":
      return <UsagePage />;
    case "doctor":
      return <DoctorPage />;
    case "settings":
      return <SettingsPage />;
    case "home":
      return <HomePage />;
  }
}

export function DaemonDashboardShell() {
  const [navOpen, setNavOpen] = useState(false);
  const navigation = useAppSelector(selectDashboardNavigation);
  const pageName =
    navigation.page === "activity"
      ? "Logs"
      : navigation.page === "chats"
        ? "Active chats"
        : navigation.page === "doctor"
          ? "Diagnostics"
          : humanizeIdentifier(navigation.page);
  const { data, error } = useGetDaemonInfoQuery(
    undefined,
    useObservationPolling(),
  );
  const status = data?.status;
  useDaemonEventsStream({ daemonStartedAtMs: status?.started_at_ms });
  const live = status !== undefined && error === undefined;

  return (
    <div className={styles.shell} data-testid="daemon-dashboard-shell">
      <aside className={styles.sidebar}>
        <div className={styles.brand}>
          <RefactIcon className={styles.brandMark} aria-hidden="true" />
          <span>
            Refact<small className={styles.brandSub}>Daemon console</small>
          </span>
        </div>
        <DashboardNav />
        <div className={styles.sidebarFooter}>
          Local daemon
          <br />
          {status
            ? `v${status.version} · ${formatUptime(status.uptime_secs)}`
            : "Not connected"}
        </div>
      </aside>
      <div className={styles.workspace}>
        <header className={styles.header}>
          <div className={styles.breadcrumb}>
            <Sheet open={navOpen} onOpenChange={setNavOpen}>
              <Sheet.Trigger asChild>
                <Button
                  className={styles.mobileMenu}
                  variant="ghost"
                  leftIcon={Menu}
                  aria-label="Open navigation"
                />
              </Sheet.Trigger>
              <Sheet.Content side="left" maxWidth="var(--rf-nav-w)">
                <Sheet.Title>Refact console</Sheet.Title>
                <Sheet.Description>Dashboard navigation</Sheet.Description>
                <DashboardNav onNavigate={() => setNavOpen(false)} />
                <Sheet.Close asChild>
                  <Button variant="ghost">Close navigation</Button>
                </Sheet.Close>
              </Sheet.Content>
            </Sheet>
            <span className={styles.muted}>Refact</span>
            <span aria-hidden="true"> / </span>
            <span>{pageName}</span>
          </div>
          <div className={styles.status} aria-live="polite">
            <QuickTour />
            <Badge tone={live ? "success" : "danger"} variant="soft">
              {live ? "Live" : "Unreachable"}
            </Badge>
            {status ? (
              <span className={styles.statusDetail}>
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

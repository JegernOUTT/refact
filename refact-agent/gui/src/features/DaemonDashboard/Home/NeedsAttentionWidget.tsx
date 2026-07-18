import { AlertTriangle, CheckCircle2 } from "lucide-react";

import { Badge, EmptyState, Icon, Surface } from "../../../components/ui";
import type { DaemonWorker } from "../../../services/refact/daemon";
import type { FailedProjectCron } from "./homeFanout";
import type { DashboardPage } from "../dashboardSlice";
import styles from "./Home.module.css";

type NeedsAttentionWidgetProps = {
  updateAvailable: boolean;
  crashedWorkers: DaemonWorker[];
  failedCrons: FailedProjectCron[];
  loading: boolean;
  onNavigate: (page: DashboardPage) => void;
};

export function NeedsAttentionWidget({
  updateAvailable,
  crashedWorkers,
  failedCrons,
  loading,
  onNavigate,
}: NeedsAttentionWidgetProps) {
  const itemCount =
    (updateAvailable ? 1 : 0) + crashedWorkers.length + failedCrons.length;
  return (
    <Surface
      as="section"
      className={styles.widget}
      radius="card"
      variant="glass"
      aria-labelledby="attention-heading"
    >
      <div className={styles.widgetHeader}>
        <div>
          <h3 id="attention-heading">Needs attention</h3>
          <p>Actionable daemon and project health signals.</p>
        </div>
        <Icon
          icon={itemCount > 0 ? AlertTriangle : CheckCircle2}
          size="md"
          tone={itemCount > 0 ? "warning" : "success"}
        />
      </div>
      {loading ? (
        <p className={styles.muted}>Checking project health…</p>
      ) : itemCount === 0 ? (
        <EmptyState
          description="Updates, workers, and scheduled runs look healthy."
          icon={CheckCircle2}
          title="All clear"
        />
      ) : (
        <ul className={styles.list}>
          {updateAvailable ? (
            <li>
              <button
                className={styles.listButton}
                onClick={() => onNavigate("settings")}
                type="button"
              >
                <span className={styles.rowCopy}>
                  <strong>Daemon update available</strong>
                  <span>Review and install it from Settings.</span>
                </span>
                <Badge tone="accent" variant="soft">
                  Settings
                </Badge>
              </button>
            </li>
          ) : null}
          {crashedWorkers.map((worker) => (
            <li key={worker.project_id}>
              <button
                className={styles.listButton}
                onClick={() => onNavigate("projects")}
                type="button"
              >
                <span className={styles.rowCopy}>
                  <strong>{worker.slug} worker crashed</strong>
                  <span>
                    {worker.last_error ?? "Restart the project worker."}
                  </span>
                </span>
                <Badge tone="danger" variant="soft">
                  Projects
                </Badge>
              </button>
            </li>
          ))}
          {failedCrons.map((cron) => (
            <li key={`${cron.projectId}:${cron.id}`}>
              <button
                className={styles.listButton}
                onClick={() => onNavigate("scheduler")}
                type="button"
              >
                <span className={styles.rowCopy}>
                  <strong>{cron.description} failed</strong>
                  <span>
                    {cron.projectSlug}
                    {cron.error ? ` · ${cron.error}` : ""}
                  </span>
                </span>
                <Badge tone="warning" variant="soft">
                  Scheduler
                </Badge>
              </button>
            </li>
          ))}
        </ul>
      )}
    </Surface>
  );
}

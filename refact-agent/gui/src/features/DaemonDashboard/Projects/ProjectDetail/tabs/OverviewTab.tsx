import { Badge, Surface } from "../../../../../components/ui";
import type { DaemonWorker } from "../../../../../services/refact/daemon";
import {
  isRagStatus,
  type CodeIntelOverview,
  type RagStatus,
} from "../../../../../services/refact/types";
import { workerPresentation } from "../../projectRagStatus";
import { codeIntelData, useProjectResource } from "../projectResource";
import { WorkerGate } from "../WorkerGate";
import { Fact, ResourceView } from "./shared";
import styles from "../ProjectDetail.module.css";

type OverviewTabProps = {
  daemonBase: string;
  worker: DaemonWorker;
  onMutated: () => void;
};

function parseRagStatus(data: unknown): RagStatus | null {
  return isRagStatus(data) ? data : null;
}

function parseOverview(data: unknown): CodeIntelOverview | null {
  return codeIntelData<CodeIntelOverview>(data);
}

function formatBytes(bytes: number): string {
  if (bytes >= 1_073_741_824) {
    return `${(bytes / 1_073_741_824).toFixed(1)} GB`;
  }
  return `${Math.max(1, Math.round(bytes / 1_048_576))} MB`;
}

function formatUptime(totalSeconds: number): string {
  if (totalSeconds < 60) return `${Math.max(0, Math.floor(totalSeconds))}s`;
  const hours = Math.floor(totalSeconds / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  return hours > 0 ? `${hours}h ${minutes}m` : `${minutes}m`;
}

function codegraphStateLabel(status: RagStatus): string {
  const codegraph = status.codegraph;
  if (!codegraph) return "unavailable";
  return codegraph.state === "indexing"
    ? `indexing · ${codegraph.queued} queued`
    : codegraph.state;
}

function vecdbStateLabel(status: RagStatus): string {
  const vecdb = status.vecdb;
  if (!vecdb) return "unavailable";
  if (status.vec_db_error) return "error";
  if (vecdb.state === "done" || vecdb.state === "cooldown") return "ready";
  return `${vecdb.state} · ${vecdb.files_unprocessed} queued`;
}

function IndexBrain({
  daemonBase,
  projectId,
}: {
  daemonBase: string;
  projectId: string;
}) {
  const ragStatus = useProjectResource(
    daemonBase,
    projectId,
    "/rag-status",
    parseRagStatus,
  );
  const overview = useProjectResource(
    daemonBase,
    projectId,
    "/code-intel/overview",
    parseOverview,
  );

  return (
    <>
      <Surface className={styles.section} radius="card" variant="glass">
        <h3 className={styles.sectionTitle}>Index brain</h3>
        <ResourceView
          errorText="Index status is unavailable."
          resource={ragStatus.resource}
        >
          {(status) => (
            <dl className={styles.factGrid}>
              <Fact label="CodeGraph" value={codegraphStateLabel(status)} />
              <Fact
                label="Nodes"
                value={status.codegraph?.counts.nodes ?? "—"}
              />
              <Fact
                label="Edges"
                value={status.codegraph?.counts.edges ?? "—"}
              />
              <Fact
                label="Files"
                value={status.codegraph?.counts.files ?? "—"}
              />
              <Fact label="VecDB" value={vecdbStateLabel(status)} />
            </dl>
          )}
        </ResourceView>
      </Surface>
      <Surface className={styles.section} radius="card" variant="glass">
        <h3 className={styles.sectionTitle}>Code graph</h3>
        <ResourceView
          errorText="Code graph overview is unavailable."
          resource={overview.resource}
        >
          {(data) => (
            <dl className={styles.factGrid}>
              <Fact label="Symbols" value={data.counts.nodes} />
              <Fact label="References" value={data.counts.edges} />
              <Fact label="Indexed files" value={data.counts.files} />
              <Fact label="Communities" value={data.community_count} />
              <Fact label="Dead code candidates" value={data.dead_code_count} />
            </dl>
          )}
        </ResourceView>
      </Surface>
    </>
  );
}

export function OverviewTab({
  daemonBase,
  worker,
  onMutated,
}: OverviewTabProps) {
  const presentation = workerPresentation(worker);

  return (
    <div className={styles.tabBody}>
      <Surface className={styles.section} radius="card" variant="glass">
        <h3 className={styles.sectionTitle}>Identity</h3>
        <dl className={styles.factGrid}>
          <Fact label="Slug" value={worker.slug} />
          <Fact label="Root" value={worker.root} mono />
          <Fact label="Project id" value={worker.project_id} mono />
          <Fact label="Pinned" value={worker.pinned ? "Yes" : "No"} />
        </dl>
      </Surface>
      <Surface className={styles.section} radius="card" variant="glass">
        <h3 className={styles.sectionTitle}>Worker</h3>
        <dl className={styles.factGrid}>
          <Fact
            label="State"
            value={
              <Badge tone={presentation.tone} variant="soft">
                {presentation.label}
              </Badge>
            }
          />
          {typeof worker.rss_bytes === "number" ? (
            <Fact label="Memory" value={formatBytes(worker.rss_bytes)} />
          ) : null}
          {typeof worker.cpu_percent === "number" ? (
            <Fact label="CPU" value={`${worker.cpu_percent.toFixed(1)}%`} />
          ) : null}
          {typeof worker.uptime_secs === "number" ? (
            <Fact label="Uptime" value={formatUptime(worker.uptime_secs)} />
          ) : null}
          <Fact label="LSP clients" value={worker.lsp_clients} />
          <Fact label="Busy chats" value={worker.busy_chats} />
          <Fact label="Exec running" value={worker.exec_running} />
        </dl>
        {worker.last_error ? (
          <p className={styles.workerError}>{worker.last_error}</p>
        ) : null}
      </Surface>
      <WorkerGate onMutated={onMutated} worker={worker}>
        <IndexBrain daemonBase={daemonBase} projectId={worker.project_id} />
      </WorkerGate>
    </div>
  );
}

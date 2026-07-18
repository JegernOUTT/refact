import { Surface } from "../../../../../components/ui";
import type { DaemonWorker } from "../../../../../services/refact/daemon";
import type {
  CodeIntelHealth,
  CodeIntelHealthFile,
  CodeIntelOverview,
} from "../../../../../services/refact/types";
import { codeIntelData, useProjectResource } from "../projectResource";
import { WorkerGate } from "../WorkerGate";
import { Fact, ResourceView } from "./shared";
import styles from "../ProjectDetail.module.css";

type HealthTabProps = {
  daemonBase: string;
  worker: DaemonWorker;
  openUrl: string;
  onMutated: () => void;
};

function parseHealth(data: unknown): CodeIntelHealth | null {
  return codeIntelData<CodeIntelHealth>(data);
}

function parseOverview(data: unknown): CodeIntelOverview | null {
  return codeIntelData<CodeIntelOverview>(data);
}

function worstHealthFiles(
  files: CodeIntelHealthFile[],
  limit: number,
): CodeIntelHealthFile[] {
  return [...files]
    .sort((left, right) => left.score - right.score)
    .slice(0, limit);
}

function HealthContent({
  daemonBase,
  projectId,
  openUrl,
}: {
  daemonBase: string;
  projectId: string;
  openUrl: string;
}) {
  const health = useProjectResource(
    daemonBase,
    projectId,
    "/code-intel/health",
    parseHealth,
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
        <h3 className={styles.sectionTitle}>Code health</h3>
        <ResourceView
          errorText="Code health is unavailable."
          resource={health.resource}
        >
          {(data) => (
            <dl className={styles.factGrid}>
              <Fact label="Grade" value={data.aggregate.grade} />
              <Fact
                label="Average score"
                value={data.aggregate.avg_score.toFixed(1)}
              />
              <Fact
                label="Maintainability"
                value={data.aggregate.avg_maintainability.toFixed(1)}
              />
              <Fact
                label="Duplication"
                value={`${data.aggregate.avg_duplication_pct.toFixed(1)}%`}
              />
              <Fact label="Biomarkers" value={data.aggregate.biomarker_count} />
              <Fact
                label="Dead code"
                value={
                  overview.resource.state === "ready" ? (
                    <a href={openUrl}>
                      {overview.resource.data.dead_code_count} candidates
                    </a>
                  ) : (
                    "—"
                  )
                }
              />
            </dl>
          )}
        </ResourceView>
      </Surface>
      <Surface className={styles.section} radius="card" variant="glass">
        <h3 className={styles.sectionTitle}>Hotspots</h3>
        <ResourceView
          errorText="Hotspots are unavailable."
          resource={health.resource}
        >
          {(data) =>
            data.files.length === 0 ? (
              <p className={styles.muted}>No analyzed files yet.</p>
            ) : (
              <ul aria-label="Health hotspots" className={styles.list}>
                {worstHealthFiles(data.files, 5).map((file) => (
                  <li className={styles.row} key={file.path}>
                    <span className={styles.rowCopy}>
                      <strong>{file.path}</strong>
                      <span>
                        {file.function_count} functions · {file.biomarker_count}{" "}
                        findings
                      </span>
                    </span>
                    <span className={styles.rowMeta}>
                      {file.grade} · {file.score.toFixed(1)}
                    </span>
                  </li>
                ))}
              </ul>
            )
          }
        </ResourceView>
      </Surface>
    </>
  );
}

export function HealthTab({
  daemonBase,
  worker,
  openUrl,
  onMutated,
}: HealthTabProps) {
  return (
    <div className={styles.tabBody}>
      <WorkerGate onMutated={onMutated} worker={worker}>
        <HealthContent
          daemonBase={daemonBase}
          openUrl={openUrl}
          projectId={worker.project_id}
        />
      </WorkerGate>
    </div>
  );
}

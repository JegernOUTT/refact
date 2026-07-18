import { Surface } from "../../../../../components/ui";
import type { DaemonWorker } from "../../../../../services/refact/daemon";
import type { TrajectoryMeta } from "../../../../../services/refact/trajectories";
import { useProjectResource } from "../projectResource";
import { WorkerGate } from "../WorkerGate";
import { ResourceView } from "./shared";
import styles from "../ProjectDetail.module.css";

const MAX_CHATS = 10;

type ChatsTabProps = {
  daemonBase: string;
  worker: DaemonWorker;
  openUrl: string;
  onMutated: () => void;
};

function parseTrajectories(data: unknown): TrajectoryMeta[] | null {
  if (Array.isArray(data)) return data as TrajectoryMeta[];
  if (
    data &&
    typeof data === "object" &&
    "items" in data &&
    Array.isArray(data.items)
  ) {
    return data.items as TrajectoryMeta[];
  }
  return null;
}

function relativeChatTime(value: string): string {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return "recently";
  const minutes = Math.max(0, Math.floor((Date.now() - timestamp) / 60_000));
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${String(minutes)}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${String(hours)}h ago`;
  return `${String(Math.floor(hours / 24))}d ago`;
}

function ChatsContent({
  daemonBase,
  projectId,
  openUrl,
}: {
  daemonBase: string;
  projectId: string;
  openUrl: string;
}) {
  const chats = useProjectResource(
    daemonBase,
    projectId,
    `/trajectories?limit=${String(MAX_CHATS)}&displayable_only=true`,
    parseTrajectories,
  );

  return (
    <Surface className={styles.section} radius="card" variant="glass">
      <h3 className={styles.sectionTitle}>Recent chats</h3>
      <ResourceView
        errorText="Recent chats are unavailable."
        resource={chats.resource}
      >
        {(items) =>
          items.length === 0 ? (
            <p className={styles.muted}>No chats in this project yet.</p>
          ) : (
            <ul aria-label="Recent chats" className={styles.list}>
              {[...items]
                .sort(
                  (left, right) =>
                    Date.parse(right.updated_at) - Date.parse(left.updated_at),
                )
                .slice(0, MAX_CHATS)
                .map((chat) => (
                  <li className={styles.row} key={chat.id}>
                    <span className={styles.rowCopy}>
                      <strong>{chat.title || "Untitled chat"}</strong>
                      <span>
                        {chat.model} · {relativeChatTime(chat.updated_at)}
                      </span>
                    </span>
                    <span className={styles.rowMeta}>
                      <a href={openUrl}>Resume</a>
                    </span>
                  </li>
                ))}
            </ul>
          )
        }
      </ResourceView>
    </Surface>
  );
}

export function ChatsTab({
  daemonBase,
  worker,
  openUrl,
  onMutated,
}: ChatsTabProps) {
  return (
    <div className={styles.tabBody}>
      <WorkerGate onMutated={onMutated} worker={worker}>
        <ChatsContent
          daemonBase={daemonBase}
          openUrl={openUrl}
          projectId={worker.project_id}
        />
      </WorkerGate>
    </div>
  );
}

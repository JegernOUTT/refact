import { Clock3, MessageSquare } from "lucide-react";

import { EmptyState, Icon, Surface } from "../../../components/ui";
import type { RecentProjectChat } from "./homeFanout";
import styles from "./Home.module.css";

type ContinueWidgetProps = {
  chats: RecentProjectChat[];
  loading: boolean;
  hadErrors: boolean;
};

function relativeTime(value: string): string {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return "Recently updated";
  const minutes = Math.max(0, Math.floor((Date.now() - timestamp) / 60_000));
  if (minutes < 1) return "Just now";
  if (minutes < 60) return `${String(minutes)}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${String(hours)}h ago`;
  return `${String(Math.floor(hours / 24))}d ago`;
}

export function ContinueWidget({
  chats,
  loading,
  hadErrors,
}: ContinueWidgetProps) {
  return (
    <Surface
      as="section"
      className={styles.widget}
      radius="card"
      variant="glass"
      aria-labelledby="continue-heading"
    >
      <div className={styles.widgetHeader}>
        <div>
          <h3 id="continue-heading">Continue recent chats</h3>
          <p>Pick up where you left off across ready projects.</p>
        </div>
        <Icon icon={Clock3} size="md" tone="muted" />
      </div>
      {loading ? (
        <p className={styles.muted}>Loading recent chats…</p>
      ) : chats.length === 0 ? (
        <EmptyState
          description={
            hadErrors
              ? "Some projects could not be checked. Open a workspace to continue."
              : "Start a chat in any ready project and it will appear here."
          }
          icon={MessageSquare}
          title="No recent chats yet"
        />
      ) : (
        <ul className={styles.list}>
          {chats.map((chat) => (
            <li key={`${chat.projectId}:${chat.id}`}>
              <a
                className={styles.listLink}
                href={`/p/${encodeURIComponent(chat.projectId)}/`}
              >
                <span className={styles.rowCopy}>
                  <strong>{chat.title}</strong>
                  <span>{chat.projectSlug}</span>
                </span>
                <span className={styles.rowMeta}>
                  {relativeTime(chat.updatedAt)}
                </span>
              </a>
            </li>
          ))}
        </ul>
      )}
    </Surface>
  );
}

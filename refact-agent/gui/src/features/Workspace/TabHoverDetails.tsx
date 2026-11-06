import { ChatActivityStatus } from "../../components/ChatActivity/ChatActivityStatus";
import { activityFromSessionState } from "../../utils/chatActivity";
import { humanizeIdentifier } from "../../utils/displayNames";
import type { TabDisplayInfo } from "./tabDisplay";
import styles from "./TabHoverDetails.module.css";

export function TabHoverDetails({ display }: { display: TabDisplayInfo }) {
  const kind =
    display.kind === "gitdiff" ? "File diff" : humanizeIdentifier(display.kind);
  const badges = [
    display.is_buddy_chat && display.kind !== "buddy" ? "Buddy" : null,
    display.is_task_chat && display.kind !== "task" ? "Task" : null,
  ].filter(Boolean);
  const rows = [
    ["File", display.filePath],
    [
      "Context",
      display.context ??
        (display.tooltip !== display.title && !display.filePath
          ? display.tooltip
          : undefined),
    ],
    ["Mode", display.mode ? humanizeIdentifier(display.mode) : undefined],
    [
      "Session",
      display.session_state
        ? humanizeIdentifier(display.session_state)
        : undefined,
    ],
    ["Task", display.taskId],
    ["Project", display.project],
    ["Branch", display.branch],
    ["Root", display.root],
  ].filter((row) => row[1]);
  return (
    <div className={styles.details}>
      <span className={styles.kind}>{[kind, ...badges].join(" · ")}</span>
      <strong className={styles.title}>{display.title}</strong>
      {(display.activity !== undefined ||
        display.session_state !== undefined) && (
        <ChatActivityStatus
          activity={
            display.activity ?? activityFromSessionState(display.session_state)
          }
        />
      )}
      {rows.length > 0 && (
        <dl className={styles.metadata}>
          {rows.map(([label, value]) => (
            <div key={label} className={styles.row}>
              <dt>{label}</dt>
              <dd>{value}</dd>
            </div>
          ))}
        </dl>
      )}
      {(display.kind === "chat" || display.unreadNotificationCount > 0) && (
        <span className={styles.unread}>
          {display.unreadNotificationCount > 0
            ? `${display.unreadNotificationCount} unread process notifications`
            : "No unread process notifications"}
        </span>
      )}
    </div>
  );
}

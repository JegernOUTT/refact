import classNames from "classnames";
import styles from "./ChatActivityStatus.module.css";
import { CHAT_ACTIVITY, type ChatActivity } from "../../utils/chatActivity";
import { Icon } from "../ui";

export function ChatActivityStatus({
  activity,
  compact = false,
  className,
}: {
  activity: ChatActivity;
  compact?: boolean;
  className?: string;
}) {
  const display = CHAT_ACTIVITY[activity];
  return (
    <span
      className={classNames(styles.status, className)}
      aria-label={display.label}
      title={display.label}
      data-chat-activity={activity}
    >
      <Icon icon={display.icon} size="sm" tone={display.tone} />
      {!compact && <span>{display.label}</span>}
    </span>
  );
}

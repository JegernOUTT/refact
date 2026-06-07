import React, { useCallback, useEffect, useMemo, useState } from "react";
import { CheckCircle2, CircleX, OctagonX, X } from "lucide-react";

import { Portal } from "../../components/Portal";
import { Badge, Button, Icon, IconButton, Surface } from "../../components/ui";
import type { IconProps } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { popBackTo, push } from "../Pages/pagesSlice";
import { switchToThread } from "../Chat/Thread";
import {
  notificationSeen,
  selectProcessCompletions,
  type ProcessCompletedNotification,
} from "./notificationsSlice";
import styles from "./Toast.module.css";

const MAX_VISIBLE_TOASTS = 3;
const AUTO_DISMISS_MS = 8000;
const SCROLL_RETRY_MS = 80;
const SCROLL_ATTEMPTS = 20;

function statusIcon(notification: ProcessCompletedNotification): IconProps["icon"] {
  if (notification.status === "killed") return OctagonX;
  if (notification.status === "exited" && notification.exitCode === 0) {
    return CheckCircle2;
  }
  return CircleX;
}

function statusTone(notification: ProcessCompletedNotification): IconProps["tone"] {
  if (notification.status === "killed") return "warning";
  if (notification.status === "exited" && notification.exitCode === 0) {
    return "success";
  }
  return "danger";
}

function exitCodeLabel(notification: ProcessCompletedNotification): string {
  return notification.exitCode === null
    ? "exit code unknown"
    : `exit ${notification.exitCode}`;
}

function scrollToProcessCard(processId: string, attempts = SCROLL_ATTEMPTS) {
  const cards = document.querySelectorAll("[data-exec-process-id]");
  const card = Array.from(cards).find(
    (item) => item.getAttribute("data-exec-process-id") === processId,
  );
  if (card) {
    card.scrollIntoView({ block: "center", behavior: "smooth" });
    return;
  }
  if (attempts <= 0) return;
  window.setTimeout(
    () => scrollToProcessCard(processId, attempts - 1),
    SCROLL_RETRY_MS,
  );
}

interface ProcessCompletedToastProps {
  notification: ProcessCompletedNotification;
  onDismiss: (notification: ProcessCompletedNotification) => void;
  onView: (notification: ProcessCompletedNotification) => void;
  onAutoDismiss: (id: string) => void;
}

const ProcessCompletedToast: React.FC<ProcessCompletedToastProps> = ({
  notification,
  onDismiss,
  onView,
  onAutoDismiss,
}) => {
  useEffect(() => {
    const timer = window.setTimeout(
      () => onAutoDismiss(notification.id),
      AUTO_DISMISS_MS,
    );
    return () => window.clearTimeout(timer);
  }, [notification.id, onAutoDismiss]);

  return (
    <Surface
      className={styles.toast}
      data-testid="process-completed-toast"
      data-notification-id={notification.id}
      variant="overlay"
    >
      <div className={styles.layout}>
        <span className={styles.legacyIcon} aria-hidden="true">
          {notification.status === "killed"
            ? "🛑"
            : notification.status === "exited" && notification.exitCode === 0
              ? "✅"
              : "❌"}
        </span>
        <Icon
          icon={statusIcon(notification)}
          tone={statusTone(notification)}
          size="lg"
          className={styles.icon}
        />
        <div className={styles.body}>
          <div className={styles.titleRow}>
            <strong className={styles.title}>
              {notification.shortDescription.trim() || "Process completed"}
            </strong>
            <Badge tone="muted">{exitCodeLabel(notification)}</Badge>
          </div>
          <span className={styles.processId}>{notification.processId}</span>
          <div className={styles.actions}>
            <Button
              size="sm"
              variant="soft"
              onClick={() => onView(notification)}
            >
              View
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => onDismiss(notification)}
            >
              Dismiss
            </Button>
          </div>
        </div>
        <IconButton
          size="sm"
          variant="ghost"
          aria-label="Dismiss process completion notification"
          onClick={() => onDismiss(notification)}
          icon={X}
        />
      </div>
    </Surface>
  );
};

export const ProcessCompletedToasts: React.FC = () => {
  const dispatch = useAppDispatch();
  const notifications = useAppSelector(selectProcessCompletions);
  const [autoDismissed, setAutoDismissed] = useState<ReadonlySet<string>>(
    () => new Set(),
  );

  useEffect(() => {
    setAutoDismissed((previous) => {
      if (previous.size === 0) return previous;
      const pendingIds = new Set(notifications.map((item) => item.id));
      const next = new Set(
        Array.from(previous).filter((id) => pendingIds.has(id)),
      );
      return next.size === previous.size ? previous : next;
    });
  }, [notifications]);

  const visibleNotifications = useMemo(
    () =>
      notifications
        .filter((notification) => !autoDismissed.has(notification.id))
        .sort((left, right) => right.receivedAt - left.receivedAt)
        .slice(0, MAX_VISIBLE_TOASTS),
    [autoDismissed, notifications],
  );

  const handleAutoDismiss = useCallback((id: string) => {
    setAutoDismissed((previous) => new Set(previous).add(id));
  }, []);

  const handleDismiss = useCallback(
    (notification: ProcessCompletedNotification) => {
      dispatch(notificationSeen({ threadId: notification.threadId }));
    },
    [dispatch],
  );

  const handleView = useCallback(
    (notification: ProcessCompletedNotification) => {
      dispatch(switchToThread({ id: notification.threadId }));
      dispatch(popBackTo({ name: "history" }));
      dispatch(push({ name: "chat" }));
      window.requestAnimationFrame(() =>
        scrollToProcessCard(notification.processId),
      );
    },
    [dispatch],
  );

  if (visibleNotifications.length === 0) return null;

  return (
    <Portal>
      <div className={styles.region} role="status" aria-live="polite">
        {visibleNotifications.map((notification) => (
          <ProcessCompletedToast
            key={notification.id}
            notification={notification}
            onDismiss={handleDismiss}
            onView={handleView}
            onAutoDismiss={handleAutoDismiss}
          />
        ))}
      </div>
    </Portal>
  );
};

export default ProcessCompletedToasts;

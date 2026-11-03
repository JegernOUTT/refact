import classNames from "classnames";
import { useCallback } from "react";
import { Badge, Tooltip } from "../../../components/ui";
import { BuddyCanvas } from "../BuddyCanvas";
import { useBuddyState } from "../hooks/useBuddyState";
import {
  setBuddyRailButton,
  toggleBuddyOpen,
  useBuddyCompanionState,
} from "./buddyCompanionStore";
import styles from "./BuddyRailButton.module.css";

export const BUDDY_RAIL_AVATAR_SIZE = 56;

export function buddyRailLabel(count: number, unread = false) {
  const label =
    count > 0
      ? `Buddy, ${count} ${count === 1 ? "needs" : "need"} you`
      : "Buddy";
  return unread ? `${label}, new reply` : label;
}

export function BuddyRailButton({ className }: { className?: string }) {
  const { open, attentionCount, unread } = useBuddyCompanionState();
  const showUnread = unread && !open;
  const buddy = useBuddyState();
  const ref = useCallback((element: HTMLButtonElement | null) => {
    setBuddyRailButton(element);
  }, []);
  return (
    <div className={classNames(styles.slot, className)}>
      <Tooltip content="Buddy">
        <button
          ref={ref}
          type="button"
          className={classNames(styles.button, "rf-pressable")}
          aria-label={buddyRailLabel(attentionCount, showUnread)}
          aria-pressed={open}
          data-buddy-rail-button=""
          onClick={toggleBuddyOpen}
        >
          <span className={styles.avatar} aria-hidden="true">
            <BuddyCanvas
              state={buddy.state}
              displaySize={BUDDY_RAIL_AVATAR_SIZE}
              showSpeech={false}
              spritePointer
            />
          </span>
        </button>
      </Tooltip>
      {attentionCount > 0 && (
        <Badge
          aria-hidden="true"
          className={classNames(styles.badge, "rf-enter-scale")}
          key={attentionCount}
          size="xs"
          tone="warning"
          data-testid="buddy-attention-badge"
        >
          {attentionCount > 99 ? "99+" : attentionCount}
        </Badge>
      )}
      {showUnread && (
        <span
          className={classNames(
            styles.unreadDot,
            attentionCount > 0 && styles.unreadDotBelow,
          )}
          aria-hidden="true"
          data-testid="buddy-unread-dot"
        />
      )}
    </div>
  );
}

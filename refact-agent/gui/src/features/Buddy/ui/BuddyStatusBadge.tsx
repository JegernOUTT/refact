import classNames from "classnames";
import { Badge, StatusDot } from "../../../components/ui";
import type { StatusDotStatus } from "../../../components/ui/StatusDot/statusTone";
import {
  buddyStatus,
  type BuddyStatusDimension,
  type BuddyStatusTone,
  type BuddyStatusValueMap,
} from "./buddyStatus";
import styles from "./BuddyStatusBadge.module.css";

const DOT_STATUS: Record<BuddyStatusTone, StatusDotStatus> = {
  default: "idle",
  success: "success",
  warning: "warning",
  danger: "error",
};

export type BuddyStatusBadgeProps<
  D extends BuddyStatusDimension = BuddyStatusDimension,
> = {
  dimension: D;
  value: BuddyStatusValueMap[D];
  label?: string;
  size?: "xs" | "sm";
  className?: string;
  testId?: string;
};

export function BuddyStatusBadge<D extends BuddyStatusDimension>({
  dimension,
  value,
  label,
  size = "xs",
  className,
  testId,
}: BuddyStatusBadgeProps<D>) {
  const entry = buddyStatus(dimension, value);
  const text = label ?? entry.label;

  if (dimension === "severity") {
    return (
      <span
        className={classNames(styles.severity, className)}
        data-dimension={dimension}
        data-value={value}
        data-tone={entry.tone}
        data-testid={testId}
      >
        <StatusDot status={DOT_STATUS[entry.tone]} />
        <span>{text}</span>
      </span>
    );
  }

  return (
    <Badge
      size={size}
      tone={entry.tone}
      className={classNames(styles.badge, className)}
      data-dimension={dimension}
      data-value={value}
      data-tone={entry.tone}
      data-testid={testId}
    >
      {text}
    </Badge>
  );
}

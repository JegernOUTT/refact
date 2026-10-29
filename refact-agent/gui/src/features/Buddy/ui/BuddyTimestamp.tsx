import React from "react";
import classNames from "classnames";
import { Tooltip } from "../../../components/ui";
import {
  formatBuddyAbsolute,
  formatBuddyDate,
  formatBuddyRelative,
  toBuddyIso,
  type BuddyTimeInput,
} from "./buddyTime";
import { BuddyInteractiveRowContext } from "./BuddyRow";
import styles from "./BuddyTimestamp.module.css";

export interface BuddyTimestampProps {
  value: BuddyTimeInput;
  mode?: "relative" | "date" | "absolute";
  prefix?: string;
  now?: number | Date;
  fallback?: React.ReactNode;
  className?: string;
  testId?: string;
}

export function BuddyTimestamp({
  value,
  mode = "relative",
  prefix,
  now,
  fallback = null,
  className,
  testId,
}: BuddyTimestampProps) {
  const interactiveRow = React.useContext(BuddyInteractiveRowContext);
  const iso = toBuddyIso(value);
  if (!iso) return <>{fallback}</>;
  const absolute = formatBuddyAbsolute(iso);
  const text =
    mode === "absolute"
      ? prefix
        ? `${prefix} ${absolute}`
        : absolute
      : mode === "date"
        ? [prefix, formatBuddyDate(iso, { now })].filter(Boolean).join(" ")
        : formatBuddyRelative(iso, { now, prefix });

  return (
    <Tooltip content={absolute}>
      <time
        dateTime={iso}
        tabIndex={interactiveRow ? undefined : 0}
        aria-label={mode === "absolute" ? undefined : `${text}, ${absolute}`}
        className={classNames(styles.time, "rf-focus-ring", className)}
        data-testid={testId}
      >
        {text}
      </time>
    </Tooltip>
  );
}

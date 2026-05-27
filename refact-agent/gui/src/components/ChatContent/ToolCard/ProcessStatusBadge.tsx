import React from "react";
import { Badge } from "@radix-ui/themes";
import classNames from "classnames";

import type { ExecProcessStatus } from "../../../services/refact/types";
import styles from "./ExecToolCard.module.css";

type ProcessStatusValue = ExecProcessStatus | (string & Record<never, never>);

type ProcessStatusBadgeProps = {
  status: ProcessStatusValue;
};

const STATUS_CLASS = {
  starting: styles.statusStarting,
  running: styles.statusRunning,
  exited: styles.statusExited,
  failed: styles.statusFailed,
  killed: styles.statusKilled,
  timed_out: styles.statusTimedOut,
} satisfies Record<ExecProcessStatus, string>;

const STATUS_CLASS_BY_VALUE: Partial<Record<string, string>> = STATUS_CLASS;

const statusLabel = (status: ProcessStatusValue): string => {
  if (!STATUS_CLASS_BY_VALUE[status]) return "unknown";
  return status === "timed_out" ? "timed out" : status;
};

export const ProcessStatusBadge: React.FC<ProcessStatusBadgeProps> = ({
  status,
}) => {
  const className = STATUS_CLASS_BY_VALUE[status] ?? styles.statusUnknown;

  return (
    <Badge
      size="1"
      variant="soft"
      className={classNames(styles.statusBadge, className)}
      data-testid={`exec-status-${status}`}
    >
      {statusLabel(status)}
    </Badge>
  );
};

export default ProcessStatusBadge;

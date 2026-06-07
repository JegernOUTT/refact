import React from "react";
import { Badge } from "../../ui";
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
  running_in_background: styles.statusRunningInBackground,
  exited: styles.statusExited,
  failed: styles.statusFailed,
  killed: styles.statusKilled,
  timed_out: styles.statusTimedOut,
} satisfies Record<ExecProcessStatus, string>;

const STATUS_CLASS_BY_VALUE: Partial<Record<string, string>> = STATUS_CLASS;

const statusLabel = (status: ProcessStatusValue): string => {
  if (!STATUS_CLASS_BY_VALUE[status]) return "unknown";
  if (status === "timed_out") return "timed out";
  if (status === "running_in_background") return "background";
  return status;
};

export const ProcessStatusBadge: React.FC<ProcessStatusBadgeProps> = ({
  status,
}) => {
  const className = STATUS_CLASS_BY_VALUE[status] ?? styles.statusUnknown;

  return (
    <Badge
      className={classNames(styles.statusBadge, className)}
      data-testid={`exec-status-${status}`}
    >
      {statusLabel(status)}
    </Badge>
  );
};

export default ProcessStatusBadge;

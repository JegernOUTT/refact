import type { ReactNode } from "react";

import { Badge, type BadgeTone } from "../../../components/ui";
import type { DoctorFinding, DoctorSeverity } from "./clientChecks";
import {
  humanizeByteMessage,
  humanizeBytes,
  parseDiskCacheDetail,
  type DiskCacheBreakdown,
} from "./diskUsage";
import styles from "./Doctor.module.css";

const DISK_CACHE_FINDING_ID = "server:disk_cache_usage";

const severityTone: Record<DoctorSeverity, BadgeTone> = {
  critical: "danger",
  warning: "warning",
  info: "muted",
};

const severityLabel: Record<DoctorSeverity, string> = {
  critical: "Critical",
  warning: "Warning",
  info: "Info",
};

type FindingCardProps = {
  finding: DoctorFinding;
  action?: ReactNode;
};

function DiskCacheTable({ breakdown }: { breakdown: DiskCacheBreakdown }) {
  const rows = [
    { label: "Worktrees", bytes: breakdown.worktrees },
    { label: "Shadow repos", bytes: breakdown.shadowRepos },
    { label: "Logs", bytes: breakdown.logs },
  ];
  return (
    <div className={styles.breakdown}>
      <table className={styles.breakdownTable}>
        <tbody>
          {rows.map((row) => (
            <tr key={row.label}>
              <th scope="row">{row.label}</th>
              <td>{humanizeBytes(row.bytes)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {breakdown.capped ? (
        <p className={styles.breakdownNote}>
          Scan was capped — actual usage may be higher.
        </p>
      ) : null}
    </div>
  );
}

export function FindingCard({ finding, action }: FindingCardProps) {
  const isDiskCacheUsage = finding.id === DISK_CACHE_FINDING_ID;
  const message = isDiskCacheUsage
    ? humanizeByteMessage(finding.message)
    : finding.message;
  const breakdown =
    isDiskCacheUsage && finding.detail !== null
      ? parseDiskCacheDetail(finding.detail)
      : null;

  return (
    <li className={styles.finding} data-testid={`finding-${finding.id}`}>
      <div className={styles.findingCopy}>
        <div className={styles.findingHeader}>
          <Badge tone={severityTone[finding.severity]} variant="soft">
            {severityLabel[finding.severity]}
          </Badge>
          <strong>{message}</strong>
        </div>
        {breakdown ? (
          <DiskCacheTable breakdown={breakdown} />
        ) : finding.detail ? (
          <p>{finding.detail}</p>
        ) : null}
      </div>
      {action ? <div className={styles.findingAction}>{action}</div> : null}
    </li>
  );
}

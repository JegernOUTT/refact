import type { ReactNode } from "react";

import { Badge, type BadgeTone } from "../../../components/ui";
import type { DoctorFinding, DoctorSeverity } from "./clientChecks";
import styles from "./Doctor.module.css";

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

export function FindingCard({ finding, action }: FindingCardProps) {
  return (
    <li className={styles.finding} data-testid={`finding-${finding.id}`}>
      <div className={styles.findingCopy}>
        <div className={styles.findingHeader}>
          <Badge tone={severityTone[finding.severity]} variant="soft">
            {severityLabel[finding.severity]}
          </Badge>
          <strong>{finding.message}</strong>
        </div>
        {finding.detail ? <p>{finding.detail}</p> : null}
      </div>
      {action ? <div className={styles.findingAction}>{action}</div> : null}
    </li>
  );
}

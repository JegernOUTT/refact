import { GitBranch } from "lucide-react";

import { Icon } from "../../../components/ui";
import styles from "./GitDock.module.css";

const SHORT_OID_LENGTH = 7;

export type BranchChipProps = {
  branch: string | null;
  headDetached: boolean;
  headOid: string | null;
};

function branchChipLabel({
  branch,
  headDetached,
  headOid,
}: BranchChipProps): string {
  if (headDetached) {
    return headOid
      ? `Detached HEAD (${headOid.slice(0, SHORT_OID_LENGTH)})`
      : "Detached HEAD";
  }
  return branch ?? "No branch";
}

export function BranchChip(props: BranchChipProps) {
  const label = branchChipLabel(props);
  return (
    <span className={styles.branchChip} title={label}>
      <Icon icon={GitBranch} size="sm" tone="muted" />
      <span className={styles.branchChipLabel}>{label}</span>
    </span>
  );
}

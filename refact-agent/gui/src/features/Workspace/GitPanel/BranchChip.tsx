import { forwardRef, type ComponentPropsWithoutRef } from "react";
import { ChevronDown, GitBranch } from "lucide-react";

import { Icon } from "../../../components/ui";
import styles from "./GitDock.module.css";

const SHORT_OID_LENGTH = 7;

export type BranchChipProps = {
  branch: string | null;
  headDetached: boolean;
  headOid: string | null;
} & Omit<ComponentPropsWithoutRef<"button">, "children">;

function branchChipLabel({
  branch,
  headDetached,
  headOid,
}: Pick<BranchChipProps, "branch" | "headDetached" | "headOid">): string {
  if (headDetached) {
    return headOid
      ? `Detached HEAD (${headOid.slice(0, SHORT_OID_LENGTH)})`
      : "Detached HEAD";
  }
  return branch ?? "No branch";
}

export const BranchChip = forwardRef<HTMLButtonElement, BranchChipProps>(
  ({ branch, headDetached, headOid, ...props }, ref) => {
    const label = branchChipLabel({ branch, headDetached, headOid });
    return (
      <button
        {...props}
        aria-label={`Git branch: ${label}`}
        className={styles.branchChip}
        ref={ref}
        title={label}
        type="button"
      >
        <Icon icon={GitBranch} size="sm" tone="muted" />
        <span className={styles.branchChipLabel}>{label}</span>
        <Icon icon={ChevronDown} size="sm" tone="muted" />
      </button>
    );
  },
);
BranchChip.displayName = "BranchChip";

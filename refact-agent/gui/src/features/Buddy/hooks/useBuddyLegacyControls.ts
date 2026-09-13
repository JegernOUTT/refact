import { useAppSelector } from "../../../hooks";
import {
  buddyOperatorIdentity,
  useGetBuddyOperatorStatusQuery,
} from "../../../services/refact/buddyOperator";
import type { BuddyControl } from "../types";

/** Exact migrated controls emitted by jobs/proactive_suggestions.rs.
 * Cosmetic dismissals, pet care and native warning navigation are not work resolution.
 */
export function isLegacyWorkControl(control: BuddyControl): boolean {
  return (
    control.action === "create_worktrees_pulse" ||
    control.action === "dismiss_suggestion"
  );
}

export function useBuddyLegacyControls() {
  const identity = useAppSelector(buddyOperatorIdentity);
  const authority = useGetBuddyOperatorStatusQuery(identity);
  return !authority.error && authority.currentData?.mode === "disabled";
}

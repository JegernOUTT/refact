import type { OperatorRecord } from "../../services/refact/buddyOperator";
import type { WorkItem } from "./operatorTypes";
import {
  CANCELLED_GROUP,
  plural,
  type DeskListMode,
  type DeskRecordGroup,
} from "./BuddyDeskModel";
import { BuddyDisclosure } from "./ui";
import styles from "./BuddyDesk.module.css";

export function BuddyDeskList({
  listMode,
  primary,
  cancelled,
  groups,
  openGroups,
  onToggleGroup,
  renderRow,
}: {
  listMode: DeskListMode;
  primary: OperatorRecord<WorkItem>[];
  cancelled: OperatorRecord<WorkItem>[];
  groups: DeskRecordGroup[];
  openGroups: ReadonlySet<string>;
  onToggleGroup: (key: string, open: boolean) => void;
  renderRow: (record: OperatorRecord<WorkItem>) => JSX.Element;
}) {
  const cancelledLabel = `Cancelled · ${plural(
    cancelled.length,
    "report",
  )} kept for history`;
  return (
    <div className={styles.scroller} data-testid="buddy-desk-scroller">
      {primary.length > 0 && listMode === "history" && (
        <ul className={styles.items} aria-label="Reports">
          {primary.map(renderRow)}
        </ul>
      )}
      {primary.length > 0 && listMode === "latest" && (
        <ul className={styles.items} aria-label="Reports">
          {groups.map((group) => (
            <li key={group.key} className={styles.group}>
              <ul className={styles.items}>{renderRow(group.head)}</ul>
              {group.others.length > 0 && (
                <BuddyDisclosure
                  className={styles.history}
                  label={`${plural(
                    group.others.length,
                    "earlier report",
                  )} for this subject`}
                  open={openGroups.has(group.key)}
                  onOpenChange={(open) => onToggleGroup(group.key, open)}
                >
                  <ul className={styles.items}>
                    {group.others.map(renderRow)}
                  </ul>
                </BuddyDisclosure>
              )}
            </li>
          ))}
        </ul>
      )}
      {cancelled.length > 0 && primary.length === 0 && (
        <section className={styles.cancelled} aria-label="Cancelled reports">
          <p className={styles.groupLabel}>{cancelledLabel}</p>
          <ul className={styles.items}>{cancelled.map(renderRow)}</ul>
        </section>
      )}
      {cancelled.length > 0 && primary.length > 0 && (
        <BuddyDisclosure
          className={styles.cancelled}
          label={cancelledLabel}
          testId="buddy-desk-cancelled"
          open={openGroups.has(CANCELLED_GROUP)}
          onOpenChange={(open) => onToggleGroup(CANCELLED_GROUP, open)}
        >
          <ul className={styles.items}>{cancelled.map(renderRow)}</ul>
        </BuddyDisclosure>
      )}
    </div>
  );
}

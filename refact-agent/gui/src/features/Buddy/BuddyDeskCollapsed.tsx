import type { Ref } from "react";
import { ClipboardList, Inbox } from "lucide-react";
import { Button } from "../../components/ui";
import { EMPTY_VIEW_COPY } from "./BuddyDeskModel";
import { BUDDY_TERMS, BuddyEmptyState, BuddySection } from "./ui";
import styles from "./BuddyDesk.module.css";

export function BuddyDeskCollapsed({
  dismissalNotice,
  expandButton,
  onExpand,
}: {
  dismissalNotice: string | null;
  expandButton: Ref<HTMLButtonElement>;
  onExpand: () => void;
}) {
  return (
    <BuddySection
      title={BUDDY_TERMS.reports}
      icon={ClipboardList}
      className={styles.desk}
      aria-label={BUDDY_TERMS.reports}
      testId="buddy-desk"
    >
      <BuddyEmptyState
        compact
        icon={Inbox}
        title={
          dismissalNotice ? (
            <span role="status">{dismissalNotice}</span>
          ) : (
            EMPTY_VIEW_COPY.needs_you
          )
        }
        action={
          <Button
            ref={expandButton}
            size="sm"
            variant="ghost"
            onClick={onExpand}
          >
            Browse reports
          </Button>
        }
      />
    </BuddySection>
  );
}

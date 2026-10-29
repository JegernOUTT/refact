import React from "react";
import { BuddyRecentChats } from "../BuddyRecentChats";
import { BuddyActivityPanel } from "../BuddyActivityPanel";
import {
  BuddyRecentErrorsPanel,
  BuddyRecentWarnings,
  type BuddyWarningRow,
  type RecentBuddyError,
} from "../BuddyRecentErrorsPanel";
import { BuddySupervision } from "../BuddySupervision";
import type { BuddyActivityEntry } from "../types";
import styles from "./BuddyActivitySection.module.css";

const LEGACY_RECENT_ERROR_LIMIT = 25;

export const BuddyActivitySection: React.FC<{
  legacyCompatibility: boolean;
  legacyRecentErrors: RecentBuddyError[];
  recentErrorRows: BuddyWarningRow[];
  activities: BuddyActivityEntry[];
  onInvestigateError: (event: RecentBuddyError) => void;
  onDismissError: (event: RecentBuddyError) => void;
  onOpenActivityChat: (chatId: string, title: string) => void;
}> = ({
  legacyCompatibility,
  legacyRecentErrors,
  recentErrorRows,
  activities,
  onInvestigateError,
  onDismissError,
  onOpenActivityChat,
}) => (
  <div
    data-section-panel="activity"
    tabIndex={-1}
    className={styles.section}
    data-testid="buddy-home-activity"
  >
    {legacyCompatibility ? (
      <BuddyRecentErrorsPanel
        recentErrors={legacyRecentErrors.slice(0, LEGACY_RECENT_ERROR_LIMIT)}
        onInvestigate={onInvestigateError}
        onDismiss={onDismissError}
      />
    ) : (
      <BuddyRecentWarnings
        rows={recentErrorRows}
        persistKey="buddy.activity.warnings"
        testId="buddy-activity-warnings"
      />
    )}
    {legacyCompatibility ? (
      <div className={styles.split}>
        <BuddyActivityPanel
          activities={activities}
          onOpenChat={onOpenActivityChat}
        />
        <BuddyRecentChats />
      </div>
    ) : (
      <BuddyRecentChats />
    )}
    <BuddySupervision />
  </div>
);

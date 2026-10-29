import React from "react";
import { ClipboardList } from "lucide-react";
import { BuddyPulseCard } from "../BuddyPulseCard";
import { BuddyOpportunitiesFeed } from "../BuddyOpportunitiesFeed";
import { BuddyDesk } from "../BuddyDesk";
import { BuddyHomeDraftReview } from "./BuddyHomeDraftReview";
import {
  BUDDY_TERMS,
  BuddyEmptyState,
  BuddyErrorCallout,
  BuddyLoading,
  BuddySection,
} from "../ui";
import styles from "./BuddyReportsSection.module.css";

export const BuddyReportsSection: React.FC<{
  legacyCompatibility: boolean;
  operatorEnabled: boolean;
  degraded: boolean;
  draftId: string | undefined;
  mappedDraft: { isFetching: boolean; error?: unknown };
  deskKey: string;
  workId: string | undefined;
  projectId: string | undefined;
}> = ({
  legacyCompatibility,
  operatorEnabled,
  degraded,
  draftId,
  mappedDraft,
  deskKey,
  workId,
  projectId,
}) => (
  <div
    data-section-panel="reports"
    tabIndex={-1}
    className={styles.section}
    data-testid="buddy-home-reports"
  >
    {legacyCompatibility && draftId && (
      <BuddyHomeDraftReview draftId={draftId} />
    )}

    {!legacyCompatibility && draftId && (
      <BuddySection title="Draft" headingLevel="h2">
        <BuddyEmptyState
          compact
          title="Draft editing sessions may expire. Saved proposals stay in Reports."
        />
        {mappedDraft.isFetching && (
          <BuddyLoading
            variant="rows"
            count={1}
            label="Finding the saved proposal"
          />
        )}
        {Boolean(mappedDraft.error) && (
          <BuddyErrorCallout
            thing="the saved proposal for this draft"
            error={mappedDraft.error}
            compact
          />
        )}
      </BuddySection>
    )}
    {operatorEnabled ? (
      <BuddyDesk key={deskKey} workId={workId} projectId={projectId} />
    ) : legacyCompatibility ? (
      <>
        <BuddyPulseCard />
        <BuddyOpportunitiesFeed />
      </>
    ) : (
      <BuddySection title={BUDDY_TERMS.reports} icon={ClipboardList}>
        {degraded ? (
          <BuddyErrorCallout
            thing="reports"
            title="Couldn't load reports. This is a connection failure, not an empty list."
          />
        ) : (
          <BuddyLoading variant="rows" count={3} label="Checking reports" />
        )}
      </BuddySection>
    )}
  </div>
);

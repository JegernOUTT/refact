import React, { useState } from "react";
import { Text } from "@radix-ui/themes";
import { BuddyOpportunityCard } from "./BuddyOpportunityCard";
import { useBuddyOpportunities } from "./hooks/useBuddyOpportunities";
import styles from "./BuddyOpportunitiesFeed.module.css";

const PAGE_SIZE = 5;

export const BuddyOpportunitiesFeed: React.FC = () => {
  const { unread } = useBuddyOpportunities();
  const [showAll, setShowAll] = useState(false);

  const visible = showAll ? unread : unread.slice(0, PAGE_SIZE);
  const hasMore = unread.length > PAGE_SIZE && !showAll;

  return (
    <div className={styles.feed} data-testid="buddy-opportunities-feed">
      <div className={styles.header}>
        <Text size="1" weight="bold" color="gray" className={styles.label}>
          OPPORTUNITIES
        </Text>
      </div>
      {unread.length === 0 ? (
        <Text size="1" className={styles.empty}>
          No opportunities right now.
        </Text>
      ) : (
        <div className={styles.list} role="list">
          {visible.map((opp) => (
            <div key={opp.id} role="listitem">
              <BuddyOpportunityCard opportunity={opp} />
            </div>
          ))}
          {hasMore && (
            <button
              type="button"
              className={styles.showMoreChip}
              onClick={() => setShowAll(true)}
            >
              Show {unread.length - PAGE_SIZE} more
            </button>
          )}
        </div>
      )}
    </div>
  );
};

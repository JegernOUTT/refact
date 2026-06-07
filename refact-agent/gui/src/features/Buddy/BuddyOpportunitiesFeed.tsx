import React from "react";
import { Text } from "@radix-ui/themes";
import { Badge, Surface } from "../../components/ui";
import { BuddyOpportunityCard } from "./BuddyOpportunityCard";
import { useBuddyOpportunities } from "./hooks/useBuddyOpportunities";
import { useAppSelector } from "../../hooks";
import { selectBuddySuggestions } from "./buddySlice";
import styles from "./BuddyOpportunitiesFeed.module.css";

export const BuddyOpportunitiesFeed: React.FC = () => {
  const { unread } = useBuddyOpportunities();
  const suggestions = useAppSelector(selectBuddySuggestions);
  const activeSuggestions = suggestions.filter(
    (suggestion) => !suggestion.dismissed,
  );
  const itemCount = unread.length + activeSuggestions.length;

  return (
    <Surface
      className={styles.feed}
      data-testid="buddy-opportunities-feed"
      radius="card"
      variant="surface-1"
    >
      <div className={styles.header}>
        <Text size="1" weight="bold" color="gray" className={styles.label}>
          OPPORTUNITIES
        </Text>
        {itemCount > 0 && (
          <Badge className={styles.count} tone="muted">
            {itemCount}
          </Badge>
        )}
      </div>
      {itemCount === 0 ? (
        <Text size="1" className={styles.empty}>
          No opportunities right now.
        </Text>
      ) : (
        <div
          className={styles.list}
          role="list"
          aria-label="Buddy opportunities"
        >
          {unread.map((opp) => (
            <div key={opp.id} className={styles.item} role="listitem">
              <BuddyOpportunityCard opportunity={opp} />
            </div>
          ))}
          {activeSuggestions.map((suggestion) => (
            <div key={suggestion.id} className={styles.item} role="listitem">
              <Text size="2" weight="bold">
                {suggestion.title}
              </Text>
              <Text size="1" color="gray">
                {suggestion.description}
              </Text>
            </div>
          ))}
        </div>
      )}
    </Surface>
  );
};

import React from "react";
import { Lightbulb } from "lucide-react";
import { BuddyOpportunityCard } from "./BuddyOpportunityCard";
import { useBuddyOpportunities } from "./hooks/useBuddyOpportunities";
import { useAppSelector } from "../../hooks";
import { selectBuddySuggestions } from "./buddySlice";
import { BuddyEmptyState, BuddyRow, BuddySection } from "./ui";
import styles from "./BuddyOpportunitiesFeed.module.css";

export const BuddyOpportunitiesFeed: React.FC = () => {
  const { unread } = useBuddyOpportunities();
  const suggestions = useAppSelector(selectBuddySuggestions);
  const activeSuggestions = suggestions.filter(
    (suggestion) => !suggestion.dismissed && suggestion.quest == null,
  );
  const itemCount = unread.length + activeSuggestions.length;

  return (
    <BuddySection
      title="Suggestions"
      icon={Lightbulb}
      count={itemCount > 0 ? itemCount : undefined}
      testId="buddy-opportunities-feed"
    >
      {itemCount === 0 ? (
        <BuddyEmptyState compact title="No suggestions right now." />
      ) : (
        <ul className={styles.list} aria-label="Suggestions">
          {unread.map((opp) => (
            <li key={opp.id} className={styles.item}>
              <BuddyOpportunityCard opportunity={opp} />
            </li>
          ))}
          {activeSuggestions.map((suggestion) => (
            <li key={suggestion.id} className={styles.item}>
              <BuddyRow
                title={suggestion.title}
                titleLines={2}
                description={suggestion.description}
              />
            </li>
          ))}
        </ul>
      )}
    </BuddySection>
  );
};

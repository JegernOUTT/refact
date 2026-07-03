import React from "react";
import classNames from "classnames";
import { Lightbulb } from "lucide-react";
import { Badge, Surface, Text } from "../../components/ui";
import { BuddyOpportunityCard } from "./BuddyOpportunityCard";
import { useBuddyOpportunities } from "./hooks/useBuddyOpportunities";
import { useAppSelector } from "../../hooks";
import { selectBuddySuggestions } from "./buddySlice";
import { BuddySectionHeader } from "./BuddySectionHeader";
import styles from "./BuddyOpportunitiesFeed.module.css";

export const BuddyOpportunitiesFeed: React.FC = () => {
  const { unread } = useBuddyOpportunities();
  const suggestions = useAppSelector(selectBuddySuggestions);
  const activeSuggestions = suggestions.filter(
    (suggestion) => !suggestion.dismissed && suggestion.quest == null,
  );
  const itemCount = unread.length + activeSuggestions.length;

  return (
    <Surface
      className={styles.feed}
      data-testid="buddy-opportunities-feed"
      animated="rise"
      radius="card"
      variant="glass"
    >
      <BuddySectionHeader
        icon={Lightbulb}
        label="Opportunities"
        badge={
          itemCount > 0 ? (
            <Badge size="xs" tone="accent" className={styles.count}>
              {itemCount}
            </Badge>
          ) : undefined
        }
      />
      {itemCount === 0 ? (
        <Text size="1" className={styles.empty}>
          No opportunities right now.
        </Text>
      ) : (
        <div
          className={classNames(styles.list, "rf-stagger")}
          role="list"
          aria-label="Buddy opportunities"
        >
          {unread.map((opp) => (
            <div
              key={opp.id}
              className={classNames(styles.item, "rf-enter-rise")}
              role="listitem"
            >
              <BuddyOpportunityCard opportunity={opp} />
            </div>
          ))}
          {activeSuggestions.map((suggestion) => (
            <div
              key={suggestion.id}
              className={classNames(styles.item, "rf-enter-rise")}
              role="listitem"
            >
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

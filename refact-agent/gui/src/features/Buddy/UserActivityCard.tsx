import React, { useMemo } from "react";
import { Flex, LoadingState, Text } from "../../components/ui";
import { Card, Chip } from "../../components/ui";
import {
  useGetUserActivityQuery,
  type UserActivityResponse,
} from "../../services/refact/buddy";
import styles from "./UserActivityCard.module.css";

type TypeCount = {
  type: string;
  count: number;
};

function makeHourBuckets(actions: UserActivityResponse["actions"]): number[] {
  const buckets = Array.from({ length: 24 }, () => 0);
  for (const action of actions) {
    const date = new Date(action.ts);
    if (Number.isNaN(date.getTime())) continue;
    buckets[date.getHours()] += 1;
  }
  return buckets;
}

function topTypes(actions: UserActivityResponse["actions"]): TypeCount[] {
  const counts = new Map<string, number>();
  for (const action of actions) {
    counts.set(action.type, (counts.get(action.type) ?? 0) + 1);
  }
  return Array.from(counts, ([type, count]) => ({ type, count }))
    .sort((a, b) => b.count - a.count || a.type.localeCompare(b.type))
    .slice(0, 3);
}

interface UserActivityCardProps {
  activity?: UserActivityResponse;
  hours?: number;
}

export const UserActivityCard: React.FC<UserActivityCardProps> = ({
  activity,
  hours = 24,
}) => {
  const { data, isLoading } = useGetUserActivityQuery(
    { hours },
    {
      skip: activity !== undefined,
      refetchOnMountOrArgChange: true,
    },
  );
  const resolvedActivity = activity ?? data;
  const buckets = useMemo(
    () => makeHourBuckets(resolvedActivity?.actions ?? []),
    [resolvedActivity?.actions],
  );
  const maxBucket = Math.max(...buckets, 1);
  const leaders = useMemo(
    () => topTypes(resolvedActivity?.actions ?? []),
    [resolvedActivity?.actions],
  );

  return (
    <Card className={styles.card} data-testid="user-activity-card">
      <Flex direction="column" gap="3">
        <Flex align="center" justify="between" gap="2">
          <Flex direction="column" gap="1">
            <Text size="2" weight="bold">
              User activity
            </Text>
            <Text size="1" color="gray">
              Last {hours} hours
            </Text>
          </Flex>
          {isLoading && activity === undefined && (
            <LoadingState label="Loading activity" variant="compact" />
          )}
        </Flex>

        <div
          className={styles.heatmap}
          aria-label="24 hour user activity heatmap"
        >
          {buckets.map((count, hour) => (
            <div
              key={hour}
              className={styles.cell}
              data-testid="user-activity-hour-cell"
              data-level={Math.ceil((count / maxBucket) * 4)}
              title={`${hour}:00 · ${count} actions`}
            />
          ))}
        </div>

        <Flex direction="column" gap="2">
          <Text size="1" weight="bold" color="gray" className={styles.label}>
            TOP ACTIONS
          </Text>
          {leaders.length === 0 ? (
            <Text size="1" color="gray">
              No activity recorded
            </Text>
          ) : (
            <div className={styles.typeList}>
              {leaders.map((leader) => (
                <Chip
                  key={leader.type}
                  className={styles.typeChip}
                  radius="chip"
                >
                  {leader.type.replace(/_/g, " ")} · {leader.count}
                </Chip>
              ))}
            </div>
          )}
        </Flex>

        <Text size="1" color="gray" className={styles.caption}>
          {resolvedActivity?.time_of_day_pattern ?? "No time pattern yet"}
        </Text>
      </Flex>
    </Card>
  );
};

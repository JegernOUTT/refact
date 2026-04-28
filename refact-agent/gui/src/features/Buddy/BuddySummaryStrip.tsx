import React from "react";
import { Text } from "@radix-ui/themes";
import classNames from "classnames";
import { formatCompactNumber } from "./buddyUtils";
import type { BuddyPetState, Stage } from "./types";
import styles from "./BuddyHome.module.css";

interface StatsSummaryData {
  totals: {
    total_calls: number;
    successful_calls: number;
    total_tokens: number;
  };
}

interface BuddySummaryStripProps {
  stage: Stage;
  xp: number;
  xpNext: number | undefined;
  xpFill: number;
  pet: BuddyPetState | undefined;
  statsData: StatsSummaryData | undefined;
  successRate: number | null;
  onViewStats: () => void;
}

export const BuddySummaryStrip: React.FC<BuddySummaryStripProps> = ({
  stage,
  xp,
  xpNext,
  xpFill,
  pet,
  statsData,
  successRate,
  onViewStats,
}) => (
  <div className={styles.summaryStrip} data-testid="buddy-summary-strip">
    <div className={styles.statItem}>
      <Text size="1" color="gray">
        Stage
      </Text>
      <Text size="2" weight="bold">
        {stage.emoji} {stage.name}
      </Text>
    </div>
    <div className={classNames(styles.statItem, styles.statItemGrow)}>
      <Text size="1" color="gray">
        Growth
      </Text>
      <div className={styles.statItemValueRow}>
        <Text size="2" weight="bold">
          {xpNext
            ? xp >= xpNext
              ? `${xpNext} / ${xpNext} · MAX`
              : `${xp} / ${xpNext}`
            : `${xp} · MAX`}
        </Text>
        <div className={styles.xpBar}>
          <div className={styles.xpFill} style={{ width: `${xpFill}%` }} />
        </div>
      </div>
    </div>
    {pet && (
      <div className={styles.statItem}>
        <Text size="1" color="gray">
          Care
        </Text>
        <Text size="2" weight="bold">
          {pet.evolution.care_score}
        </Text>
      </div>
    )}
    {pet && (
      <div className={styles.statItem}>
        <Text size="1" color="gray">
          Neglect
        </Text>
        <Text size="2" weight="bold">
          {pet.evolution.neglect_score}
        </Text>
      </div>
    )}
    {statsData && (
      <>
        <div className={styles.statItemDivider} aria-hidden />
        <div className={styles.statItem}>
          <Text size="1" color="gray">
            Messages
          </Text>
          <Text size="2" weight="bold">
            {formatCompactNumber(statsData.totals.total_calls)}
          </Text>
        </div>
        <div className={styles.statItem}>
          <Text size="1" color="gray">
            Tokens
          </Text>
          <Text size="2" weight="bold">
            {formatCompactNumber(statsData.totals.total_tokens)}
          </Text>
        </div>
        <div className={styles.statItem}>
          <Text size="1" color="gray">
            Success
          </Text>
          <Text size="2" weight="bold">
            {successRate ?? 0}%
          </Text>
        </div>
      </>
    )}
    <div className={styles.statSpacer} aria-hidden />
    {statsData && (
      <button
        type="button"
        className={classNames(styles.chip, styles.chipGhost)}
        onClick={onViewStats}
      >
        View Full Stats →
      </button>
    )}
  </div>
);

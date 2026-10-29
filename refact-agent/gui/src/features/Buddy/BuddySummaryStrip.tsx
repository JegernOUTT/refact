import React from "react";
import { ArrowRight } from "lucide-react";
import { Button, Icon, Text } from "../../components/ui";
import { BuddySurface } from "./ui";
import { formatCompactNumber, xpDisplay } from "./buddyUtils";
import { stageIcon } from "./buddyIcons";
import type { BuddyPetState, Palette, Stage } from "./types";
import styles from "./BuddySummaryStrip.module.css";

interface StatsSummaryData {
  totals: {
    total_calls: number;
    successful_calls: number;
    total_tokens: number;
  };
}

interface BuddySummaryStripProps {
  name: string;
  palette: Palette;
  stage: Stage;
  stageIndex: number;
  xp: number;
  xpNext: number | undefined;
  xpFill: number;
  atMaxStage: boolean;
  pet: BuddyPetState | undefined;
  statsData: StatsSummaryData | undefined;
  successRate: number | null;
  onViewStats: () => void;
}

const StatItem: React.FC<{ label: string; value: React.ReactNode }> = ({
  label,
  value,
}) => (
  <div className={styles.stat}>
    <Text size="1" color="gray" className={styles.statLabel}>
      {label}
    </Text>
    <Text size="2" weight="bold" className={styles.statValue}>
      {value}
    </Text>
  </div>
);

export const BuddySummaryStrip: React.FC<BuddySummaryStripProps> = ({
  name,
  stage,
  stageIndex,
  xp,
  xpNext,
  xpFill,
  atMaxStage,
  pet,
  statsData,
  successRate,
  onViewStats,
}) => {
  const xpLabel = atMaxStage
    ? `${xp} XP · max stage`
    : xpDisplay(xp, xpNext, atMaxStage);

  return (
    <BuddySurface
      level="inset"
      className={styles.strip}
      testId="buddy-summary-strip"
    >
      <div className={styles.identity}>
        <span className={styles.avatar} aria-hidden>
          <Icon icon={stageIcon(stageIndex)} size="lg" />
        </span>
        <div className={styles.identityText}>
          <span className={styles.nameRow}>
            <Text size="2" weight="bold" className={styles.name}>
              {name}
            </Text>
            <Text size="1" color="gray" className={styles.stageMeta}>
              {stage.name} · {xpLabel}
            </Text>
          </span>
          {!atMaxStage && (
            <progress
              className={styles.xpBar}
              aria-label="Progress to next stage"
              max={100}
              value={Math.round(xpFill)}
            />
          )}
        </div>
      </div>
      <div className={styles.stats}>
        {pet && (
          <StatItem
            label="Care"
            value={formatCompactNumber(pet.evolution.care_score)}
          />
        )}
        {statsData && (
          <>
            <StatItem
              label="Messages"
              value={formatCompactNumber(statsData.totals.total_calls)}
            />
            <StatItem
              label="Tokens"
              value={formatCompactNumber(statsData.totals.total_tokens)}
            />
            <StatItem label="Success" value={`${successRate ?? 0}%`} />
          </>
        )}
        {statsData && (
          <Button
            type="button"
            size="sm"
            variant="plain"
            rightIcon={ArrowRight}
            onClick={onViewStats}
            className={styles.statsLink}
          >
            View stats
          </Button>
        )}
      </div>
    </BuddySurface>
  );
};

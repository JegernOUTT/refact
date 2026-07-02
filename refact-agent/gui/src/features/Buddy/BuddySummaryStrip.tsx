import React from "react";
import { ArrowRight } from "lucide-react";
import { Button, Icon, Surface, Text } from "../../components/ui";
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

/**
 * Identity & stats bar: compact avatar + name + stage/XP cluster on the
 * left, key counters in the middle, "View stats" on the right. Replaces
 * the old wide hero identity column.
 */
export const BuddySummaryStrip: React.FC<BuddySummaryStripProps> = ({
  name,
  palette,
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
  const xpLabel = xpDisplay(xp, xpNext, atMaxStage);
  const displayedXpFill = atMaxStage ? 100 : xpFill;

  return (
    <Surface
      className={styles.strip}
      data-testid="buddy-summary-strip"
      radius="card"
      variant="glass"
      animated="rise"
    >
      <div
        className={styles.identity}
        style={{ "--buddy-tint": palette.body } as React.CSSProperties}
      >
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
          <div className={styles.xpBar}>
            <div
              className={styles.xpFill}
              style={
                {
                  "--buddy-xp-fill": `${displayedXpFill}%`,
                } as React.CSSProperties
              }
            />
          </div>
        </div>
      </div>
      {pet && (
        <>
          <div className={styles.divider} aria-hidden />
          <StatItem label="Care" value={pet.evolution.care_score} />
          <StatItem label="Neglect" value={pet.evolution.neglect_score} />
        </>
      )}
      {statsData && (
        <>
          <div className={styles.divider} aria-hidden />
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
      <span className={styles.spacer} aria-hidden />
      {statsData && (
        <Button
          type="button"
          size="sm"
          variant="ghost"
          rightIcon={ArrowRight}
          onClick={onViewStats}
        >
          View stats
        </Button>
      )}
    </Surface>
  );
};

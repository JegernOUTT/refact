import React from "react";
import { Gamepad2, RotateCcw } from "lucide-react";
import { Button } from "../../../components/ui";
import { BuddySection } from "../ui";
import { BuddySummaryStrip } from "../BuddySummaryStrip";
import styles from "./BuddyPlaySection.module.css";

export const BuddyPlaySection: React.FC<{
  world: React.ReactNode;
  degraded: boolean;
  onResetWorld: () => void;
  summary: React.ComponentProps<typeof BuddySummaryStrip>;
}> = ({ world, degraded, onResetWorld, summary }) => (
  <div
    data-section-panel="play"
    tabIndex={-1}
    className={styles.section}
    data-testid="buddy-home-play"
  >
    <BuddySection
      title="Play"
      icon={Gamepad2}
      testId="buddy-play-section"
      bodyClassName={styles.body}
      action={
        <Button
          type="button"
          size="sm"
          variant="ghost"
          leftIcon={RotateCcw}
          onClick={onResetWorld}
        >
          Reset scene
        </Button>
      }
    >
      <div
        className={styles.scene}
        id="buddy-home-hero"
        data-testid="buddy-home-hero"
      >
        {world}
      </div>
      {!degraded && <BuddySummaryStrip {...summary} />}
    </BuddySection>
  </div>
);

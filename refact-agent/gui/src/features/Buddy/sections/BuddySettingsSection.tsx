import React, { useCallback, useRef } from "react";
import { Wrench } from "lucide-react";
import classNames from "classnames";
import { Button } from "../../../components/ui";
import { BuddySettingsPanel } from "../BuddySettingsPanel";
import { BuddyAppliedChangesCard } from "../BuddyAppliedChangesCard";
import { BuddyPersonalityPanel } from "../BuddyPersonalityPanel";
import { BuddySection, buddyName } from "../ui";
import { useBuddyNeedRows } from "./buddyNeedRows";
import { SETUP_MODES } from "../../Setup/setupModes";
import type {
  BuddyControl,
  BuddyPersonalityProfile,
  BuddyPetState,
  BuddyQuest,
  BuddySettings,
} from "../types";
import styles from "./BuddySettingsSection.module.css";

function sentenceCase(label: string): string {
  return label
    .split(" ")
    .map((word, index) =>
      index === 0 || /[A-Z].*[A-Z.]/.test(word) ? word : word.toLowerCase(),
    )
    .join(" ");
}

export const BuddySettingsSection: React.FC<{
  personality: BuddyPersonalityProfile | undefined;
  pet: BuddyPetState | undefined;
  unlockedSkills: string[];
  activeQuest: BuddyQuest | null;
  name: string;
  settings: BuddySettings | undefined;
  degraded: boolean;
  operatorNeedsRetry: boolean;
  onQuestControl: (control: BuddyControl) => Promise<void>;
  onReroll: () => Promise<void>;
  onRunMode: (mode: string) => void;
}> = ({
  personality,
  pet,
  unlockedSkills,
  activeQuest,
  name,
  settings,
  degraded,
  operatorNeedsRetry,
  onQuestControl: handleQuestControl,
  onReroll: handleReroll,
  onRunMode: handleRunMode,
}) => {
  const needRows = useBuddyNeedRows(pet);
  const settingsRef = useRef<HTMLDivElement>(null);
  const displayName = buddyName(name);
  const handleOpenSettings = useCallback(() => {
    const target = settingsRef.current?.querySelector<HTMLElement>(
      "[data-testid='buddy-toggle-proactive']",
    );
    target?.scrollIntoView({ block: "center" });
    target?.focus();
  }, []);
  return (
    <div
      data-section-panel="settings"
      tabIndex={-1}
      className={classNames(styles.section, styles.settingsSection)}
      data-testid="buddy-home-settings-section"
    >
      <BuddyPersonalityPanel
        personality={personality}
        needRows={needRows}
        unlockedSkills={unlockedSkills}
        activeQuest={activeQuest}
        name={displayName}
        settings={settings}
        onQuestControl={(control) => void handleQuestControl(control)}
        onReroll={() => void handleReroll()}
        onOpenSettings={handleOpenSettings}
      />
      <BuddyAppliedChangesCard />
      <div ref={settingsRef} className={styles.settingsSection}>
        <BuddySettingsPanel />
      </div>
      <BuddySection
        title="Project setup"
        icon={Wrench}
        description={`Start a guided chat where ${displayName} sets up this project.`}
        testId="buddy-setup-section"
      >
        <div
          className={styles.setupGrid}
          data-disabled={degraded ? "true" : undefined}
          data-testid="buddy-setup-strip"
        >
          {SETUP_MODES.map((m) => (
            <Button
              key={m.mode}
              type="button"
              size="sm"
              variant={m.mode === "setup" ? "primary" : "ghost"}
              disabled={operatorNeedsRetry}
              onClick={() => handleRunMode(m.mode)}
            >
              {sentenceCase(m.label)}
            </Button>
          ))}
        </div>
        {degraded ? (
          <p className={styles.setupNotice}>
            Setup runs need the engine, which just stopped answering.
          </p>
        ) : null}
      </BuddySection>
    </div>
  );
};

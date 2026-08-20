import React, { useMemo } from "react";
import { ChevronDown, ChevronUp, Rabbit } from "lucide-react";
import { Button, Icon } from "../../../../components/ui";
import { BuddyDashboardScene } from "../../../Buddy/BuddyDashboardScene";
import { useAppSelector } from "../../../../hooks";
import { useDashboardCollapseState } from "../../hooks/useDashboardCollapseState";
import { selectContinueItems, isActiveStatus } from "../../streamSelectors";
import styles from "./HeroBlock.module.css";

export const HeroBlock: React.FC = () => {
  const { collapsed, toggle } = useDashboardCollapseState();
  const continueItems = useAppSelector(selectContinueItems);

  const activeCount = useMemo(
    () =>
      continueItems.filter(
        (item) => isActiveStatus(item.status) || (item.agentsActive ?? 0) > 0,
      ).length,
    [continueItems],
  );

  const isCollapsed = collapsed.buddy;
  const statusText =
    activeCount > 0
      ? "Refact is working on your workspace"
      : "Refact is idle — pick something up below";

  return (
    <section
      className={styles.hero}
      data-collapsed={isCollapsed || undefined}
      aria-label="Refact buddy"
    >
      <div className={styles.sceneWrap}>
        <div className={styles.sceneInner}>
          <BuddyDashboardScene />
        </div>
      </div>

      <div className={styles.strip}>
        <Icon icon={Rabbit} size="md" tone="accent" />
        <span className={styles.statusText}>{statusText}</span>
        {activeCount > 0 ? (
          <span className={styles.activeChip}>{activeCount} active</span>
        ) : null}
        <Button
          variant="ghost"
          size="sm"
          className={styles.toggle}
          onClick={() => toggle("buddy")}
          aria-expanded={!isCollapsed}
          rightIcon={isCollapsed ? ChevronDown : ChevronUp}
        >
          {isCollapsed ? "Show" : "Hide"}
        </Button>
      </div>
    </section>
  );
};

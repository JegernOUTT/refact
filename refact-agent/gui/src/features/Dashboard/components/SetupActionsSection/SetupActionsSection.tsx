import React, { useCallback } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
import { Button } from "../../../../components/ui";
import { DashboardText } from "../DashboardPrimitives";
import { useAppDispatch } from "../../../../hooks";
import { openChatInModeAndStart } from "../../../Chat/Thread/actions";
import { SETUP_MODES } from "../../../Setup/setupModes";
import { CollapsePanel } from "../../../../components/shared/CollapsePanel";
import styles from "./SetupActionsSection.module.css";

const SETUP_ACTIONS = SETUP_MODES.filter((m) => m.mode !== "setup");

type Props = {
  collapsed: boolean;
  onToggleCollapsed: () => void;
};

export const SetupActionsSection: React.FC<Props> = ({
  collapsed,
  onToggleCollapsed,
}) => {
  const dispatch = useAppDispatch();

  const openSetupChat = useCallback(
    (mode: string) => {
      void dispatch(openChatInModeAndStart({ mode }));
    },
    [dispatch],
  );

  return (
    <div className={styles.section} data-collapsed={collapsed || undefined}>
      <Button
        variant="plain"
        size="sm"
        className={styles.headerToggle}
        onClick={onToggleCollapsed}
        aria-expanded={!collapsed}
        rightIcon={collapsed ? ChevronDown : ChevronUp}
      >
        <DashboardText size="1" weight="bold" tone="muted" className={styles.label}>
          PROJECT SETUP
        </DashboardText>
      </Button>
      <CollapsePanel collapsed={collapsed}>
        <div className={styles.buttons}>
          {SETUP_ACTIONS.map((action) => (
            <Button
              key={action.mode}
              variant="soft"
              size="sm"
              className={styles.button}
              onClick={() => openSetupChat(action.mode)}
            >
              {action.label}
            </Button>
          ))}
        </div>
      </CollapsePanel>
    </div>
  );
};

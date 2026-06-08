import React, { useCallback } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
import { Button } from "../../../../components/ui";
import { DashboardText } from "../DashboardPrimitives";
import { CollapsePanel } from "../../../../components/shared/CollapsePanel";
import { useAppDispatch } from "../../../../hooks";
import { switchToThread } from "../../../Chat/Thread";
import { popBackTo, push } from "../../../Pages/pagesSlice";
import { useGetChatModesQuery } from "../../../../services/refact/chatModes";
import { OpenTabCard } from "./OpenTabCard";
import type { OpenTabData, DashboardBreakpoint } from "../../types";
import styles from "./OpenSection.module.css";

type OpenSectionProps = {
  tabs: OpenTabData[];
  breakpoint: DashboardBreakpoint;
  collapsed: boolean;
  onToggleCollapsed: () => void;
};

export const OpenSection: React.FC<OpenSectionProps> = ({
  tabs,
  breakpoint,
  collapsed,
  onToggleCollapsed,
}) => {
  const dispatch = useAppDispatch();
  const { data: modesData } = useGetChatModesQuery(undefined);

  const handleTabClick = useCallback(
    (tabId: string) => {
      dispatch(switchToThread({ id: tabId }));
      dispatch(popBackTo({ name: "history" }));
      dispatch(push({ name: "chat" }));
    },
    [dispatch],
  );

  if (tabs.length === 0) return null;

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
        <DashboardText
          size="1"
          weight="bold"
          tone="muted"
          className={styles.label}
        >
          OPEN
        </DashboardText>
        <DashboardText size="1" tone="muted">
          {tabs.length} open
        </DashboardText>
      </Button>
      <CollapsePanel collapsed={collapsed}>
        <div className={styles.scrollWrapper} data-breakpoint={breakpoint}>
          <div className={styles.grid} data-breakpoint={breakpoint}>
            {tabs.map((tab) => {
              const modeInfo = modesData?.modes.find((m) => m.id === tab.mode);
              const modeLabel = modeInfo?.title ?? tab.mode;
              return (
                <OpenTabCard
                  key={tab.id}
                  tab={tab}
                  breakpoint={breakpoint}
                  modeLabel={modeLabel}
                  onClick={() => handleTabClick(tab.id)}
                />
              );
            })}
          </div>
        </div>
      </CollapsePanel>
    </div>
  );
};

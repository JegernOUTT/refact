import React, { useCallback } from "react";
import { Text } from "@radix-ui/themes";
import { DrawingPinFilledIcon, ChevronDownIcon, ChevronUpIcon } from "@radix-ui/react-icons";
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
    <div className={styles.section}>
      <button
        type="button"
        className={styles.headerToggle}
        onClick={onToggleCollapsed}
        aria-expanded={!collapsed}
      >
        <Text size="1" weight="bold" color="gray" className={styles.label}>
          <DrawingPinFilledIcon width={10} height={10} style={{ display: "inline", verticalAlign: "middle" }} /> OPEN
        </Text>
        <Text size="1" color="gray">{tabs.length} open</Text>
        {collapsed ? (
          <ChevronDownIcon width={12} height={12} color="var(--gray-9)" />
        ) : (
          <ChevronUpIcon width={12} height={12} color="var(--gray-9)" />
        )}
      </button>
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

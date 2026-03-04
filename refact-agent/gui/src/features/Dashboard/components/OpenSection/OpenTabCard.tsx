import React from "react";
import { Badge, Text } from "@radix-ui/themes";
import { StatusDot } from "../../../../components/StatusDot";
import { getStatusFromSessionState } from "../../../../utils/sessionStatus";
import { getModeColor } from "../../../../utils/modeColors";
import { TodoProgress } from "./TodoProgress";
import { DotTrail } from "../DotTrail/DotTrail";
import type { OpenTabData, DashboardBreakpoint } from "../../types";
import styles from "./OpenTabCard.module.css";

type OpenTabCardProps = {
  tab: OpenTabData;
  breakpoint: DashboardBreakpoint;
  modeLabel?: string;
  onClick: () => void;
};

export const OpenTabCard: React.FC<OpenTabCardProps> = ({
  tab,
  breakpoint,
  modeLabel,
  onClick,
}) => {
  const statusState = getStatusFromSessionState(tab.session_state);
  const isActive = statusState === "in_progress";

  const showDotTrail = breakpoint !== "narrow";
  const showTodos = breakpoint === "wide";

  return (
    <button
      type="button"
      className={styles.card}
      data-active={isActive || undefined}
      onClick={onClick}
    >
      <div className={styles.header}>
        <StatusDot state={statusState} size="small" />
        <Text size="2" weight="medium" truncate className={styles.title}>
          {tab.title}
        </Text>
        {modeLabel && (
          <Badge
            size="1"
            color={getModeColor(tab.mode)}
            variant="soft"
            className={styles.modeBadge}
          >
            {modeLabel}
          </Badge>
        )}
      </div>
      {showDotTrail && tab.treeNode && tab.treeNode.children.length > 0 && (
        <DotTrail node={tab.treeNode} breakpoint={breakpoint} />
      )}
      {showTodos && tab.todos.length > 0 && (
        <TodoProgress todos={tab.todos} breakpoint={breakpoint} />
      )}
    </button>
  );
};

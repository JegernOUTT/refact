import React from "react";
import classNames from "classnames";
import { LoaderCircle } from "lucide-react";
import { ToolCard as KitToolCard } from "../../ui";
import { Icon } from "../../ui/Icon";
import { useDelayedUnmount } from "../../shared/useDelayedUnmount";
import { ToolCallTooltip } from "./ToolCallTooltip";
import { ToolCall } from "../../../services/refact/types";
import styles from "./ToolCard.module.css";

export type ToolStatus = "running" | "success" | "error";

export interface ToolCardProps {
  icon: React.ReactNode;
  summary: React.ReactNode;
  meta?: React.ReactNode;
  status: ToolStatus;
  isOpen: boolean;
  onToggle: () => void;
  children?: React.ReactNode;
  className?: string;
  animate?: boolean;
  toolCall?: ToolCall;
}

const ToolCardInner: React.FC<ToolCardProps> = ({
  icon,
  summary,
  meta,
  status,
  isOpen,
  onToggle,
  children,
  className,
  animate = true,
  toolCall,
}) => {
  const { shouldRender, isAnimatingOpen } = useDelayedUnmount(
    isOpen,
    200,
    animate,
  );
  const renderedOpen = animate ? isAnimatingOpen : isOpen;

  const title = (
    <span className={styles.titleRow}>
      <span className={styles.iconWrapper}>
        {status === "running" ? <Icon icon={LoaderCircle} /> : icon}
      </span>
      <span className={styles.summary}>{summary}</span>
      {meta && <span className={styles.meta}>{meta}</span>}
    </span>
  );

  const card = (
    <KitToolCard
      className={classNames(
        "rf-enter",
        styles.card,
        status === "running" && "rf-status-pulse",
        status === "running" && styles.running,
        status === "success" && styles.completed,
        status === "error" && styles.error,
        className,
      )}
      open={renderedOpen}
      onOpenChange={onToggle}
      status={status}
      title={title}
    >
      {shouldRender ? <div className={styles.content}>{children}</div> : null}
    </KitToolCard>
  );

  return toolCall ? (
    <ToolCallTooltip toolCall={toolCall}>{card}</ToolCallTooltip>
  ) : (
    card
  );
};

ToolCardInner.displayName = "ToolCard";

export const ToolCard = React.memo(ToolCardInner);

export default ToolCard;

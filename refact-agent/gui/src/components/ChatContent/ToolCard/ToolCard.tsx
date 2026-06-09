import React from "react";
import classNames from "classnames";
import { LoaderCircle } from "lucide-react";
import { ToolCard as KitToolCard } from "../../ui";
import { Icon } from "../../ui/Icon";
import {
  COLLAPSE_ANIMATION_MS,
  useDelayedUnmount,
} from "../../shared/useDelayedUnmount";
import { ToolCallTooltip } from "./ToolCallTooltip";
import {
  useChatScrollAnchor,
  usePrepareChatScrollAnchor,
} from "../useChatScrollAnchor";
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
  const preserveScrollAnchor = useChatScrollAnchor();
  const prepareScrollAnchor = usePrepareChatScrollAnchor();
  const { shouldRender } = useDelayedUnmount(
    isOpen,
    COLLAPSE_ANIMATION_MS,
    animate,
  );
  const shouldRenderBody = isOpen || shouldRender;
  const handleToggle = React.useCallback(() => {
    preserveScrollAnchor(onToggle);
  }, [onToggle, preserveScrollAnchor]);

  const title = (
    <span
      className={classNames(
        styles.titleRow,
        status === "running" && "rf-active-pulse",
      )}
    >
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
        status === "running" && styles.running,
        status === "success" && styles.completed,
        status === "error" && styles.error,
        className,
      )}
      open={isOpen}
      onPointerDownCapture={prepareScrollAnchor}
      onMouseDownCapture={prepareScrollAnchor}
      onKeyDownCapture={prepareScrollAnchor}
      onOpenChange={handleToggle}
      status={status}
      title={title}
    >
      {shouldRenderBody ? (
        <div className={styles.content}>{children}</div>
      ) : null}
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

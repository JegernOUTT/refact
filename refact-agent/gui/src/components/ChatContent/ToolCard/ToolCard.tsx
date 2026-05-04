import React from "react";
import { Flex, Text, Spinner } from "@radix-ui/themes";
import classNames from "classnames";
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

export const ToolCard: React.FC<ToolCardProps> = React.memo(
  ({
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

    const header = (
      <Flex className={styles.header} align="center" gap="2" onClick={onToggle}>
        <span className={styles.iconWrapper}>
          {status === "running" ? <Spinner size="1" /> : icon}
        </span>

        <Text size="1" className={styles.summary}>
          {summary}
        </Text>

        {meta && (
          <Text size="1" color="gray" className={styles.meta}>
            {meta}
          </Text>
        )}
      </Flex>
    );

    return (
      <div
        className={classNames(
          styles.card,
          status === "running" && styles.running,
          status === "success" && styles.completed,
          status === "error" && styles.error,
          className,
        )}
      >
        {toolCall ? (
          <ToolCallTooltip toolCall={toolCall}>{header}</ToolCallTooltip>
        ) : (
          header
        )}

        {shouldRender && children && (
          <div
            className={classNames(
              styles.contentWrapper,
              isAnimatingOpen && styles.contentWrapperOpen,
              !animate && styles.noTransition,
            )}
          >
            <div className={styles.contentInner}>
              <div className={styles.content}>{children}</div>
            </div>
          </div>
        )}
      </div>
    );
  },
);

export default ToolCard;

import React from "react";
import { Flex, Text, Spinner } from "@radix-ui/themes";
import classNames from "classnames";
import { useDelayedUnmount } from "../../shared/useDelayedUnmount";
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
}

export const ToolCard: React.FC<ToolCardProps> = ({
  icon,
  summary,
  meta,
  status,
  isOpen,
  onToggle,
  children,
  className,
}) => {
  const { shouldRender, isAnimatingOpen } = useDelayedUnmount(isOpen, 200);

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

      {shouldRender && children && (
        <div
          className={classNames(
            styles.contentWrapper,
            isAnimatingOpen && styles.contentWrapperOpen,
          )}
        >
          <div className={styles.contentInner}>
            <div className={styles.content}>{children}</div>
          </div>
        </div>
      )}
    </div>
  );
};

export default ToolCard;

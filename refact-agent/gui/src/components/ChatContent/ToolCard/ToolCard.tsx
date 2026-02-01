import React from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Flex, Text, Spinner } from "@radix-ui/themes";
import classNames from "classnames";
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
  return (
    <div
      className={classNames(
        styles.card,
        status === "running" && styles.running,
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

      <AnimatePresence initial={false}>
        {isOpen && children && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.2, ease: "easeInOut" }}
            className={styles.contentWrapper}
          >
            <div className={styles.content}>{children}</div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
};

export default ToolCard;

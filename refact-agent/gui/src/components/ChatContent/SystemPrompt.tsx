import React, { useState, useCallback } from "react";
import { Box, Text, Flex } from "@radix-ui/themes";
import classNames from "classnames";
import { BookOpen } from "lucide-react";
import { Markdown } from "../Markdown";
import { Icon } from "../ui";
import { useDelayedUnmount } from "../shared/useDelayedUnmount";
import styles from "./SystemPrompt.module.css";

export const SystemPrompt: React.FC<{
  content: string;
}> = ({ content }) => {
  const [isOpen, setIsOpen] = useState(false);
  const { shouldRender, isAnimatingOpen } = useDelayedUnmount(isOpen, 200);

  const handleToggle = useCallback(() => {
    setIsOpen((prev) => !prev);
  }, []);

  if (!content.trim()) return null;

  return (
    <div className={`${styles.card} rf-enter-rise`}>
      <Flex
        className={styles.header}
        align="center"
        gap="2"
        onClick={handleToggle}
      >
        <span className={styles.icon}>
          <Icon icon={BookOpen} size="sm" tone="muted" />
        </span>
        <Text size="1" className={styles.summary}>
          System prompt
        </Text>
      </Flex>

      {shouldRender && (
        <div
          className={classNames(
            "rf-expand-grid",
            isAnimatingOpen && "is-open",
            styles.contentWrapper,
            isAnimatingOpen && styles.contentWrapperOpen,
          )}
        >
          <div className={styles.contentInner}>
            <Box className={styles.content}>
              <Markdown>{content}</Markdown>
            </Box>
          </div>
        </div>
      )}
    </div>
  );
};

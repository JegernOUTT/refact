import React, { useState, useCallback } from "react";
import { Box, Text, Flex } from "@radix-ui/themes";
import classNames from "classnames";
import { ReaderIcon } from "@radix-ui/react-icons";
import { Markdown } from "../Markdown";
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
    <div className={styles.card}>
      <Flex
        className={styles.header}
        align="center"
        gap="2"
        onClick={handleToggle}
      >
        <span className={styles.icon}>
          <ReaderIcon />
        </span>
        <Text size="1" className={styles.summary}>
          System prompt
        </Text>
      </Flex>

      {shouldRender && (
        <div
          className={classNames(
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

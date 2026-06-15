import React, { useState, useCallback } from "react";
import { Box, Text } from "@radix-ui/themes";
import { BookOpen } from "lucide-react";
import { Markdown } from "../Markdown";
import { Icon } from "../ui";
import { AnimatedCollapsible } from "./shared/AnimatedCollapsible";
import styles from "./SystemPrompt.module.css";

export const SystemPrompt: React.FC<{
  content: string;
}> = ({ content }) => {
  const [isOpen, setIsOpen] = useState(false);

  const handleOpenChange = useCallback((open: boolean) => {
    setIsOpen(open);
  }, []);

  if (!content.trim()) return null;

  return (
    <AnimatedCollapsible
      className={`${styles.card} rf-enter-rise`}
      header={
        <Text size="1" className={styles.summary}>
          System prompt
        </Text>
      }
      icon={
        <span className={styles.icon}>
          <Icon icon={BookOpen} size="sm" tone="muted" />
        </span>
      }
      open={isOpen}
      onOpenChange={handleOpenChange}
      variant="compact"
    >
      <Box className={styles.content}>
        <Markdown>{content}</Markdown>
      </Box>
    </AnimatedCollapsible>
  );
};

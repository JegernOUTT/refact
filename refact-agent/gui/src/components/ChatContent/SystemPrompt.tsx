import React, { useState, useCallback } from "react";
import { Box, Text, Flex } from "@radix-ui/themes";
import { motion, AnimatePresence } from "framer-motion";
import { ReaderIcon } from "@radix-ui/react-icons";
import { Markdown } from "../Markdown";
import styles from "./SystemPrompt.module.css";

export const SystemPrompt: React.FC<{
  content: string;
}> = ({ content }) => {
  const [isOpen, setIsOpen] = useState(false);

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

      <AnimatePresence initial={false}>
        {isOpen && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.2, ease: "easeInOut" }}
            className={styles.contentWrapper}
          >
            <Box className={styles.content}>
              <Markdown>{content}</Markdown>
            </Box>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
};

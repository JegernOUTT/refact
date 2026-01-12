import React from "react";
import { Flex, Container, Box } from "@radix-ui/themes";
import styles from "./ChatLoading.module.css";

export const ChatLoading: React.FC = () => {
  return (
    <Container>
      <Flex
        direction="column"
        align="center"
        justify="center"
        gap="4"
        py="9"
        className={styles.container}
      >
        <Box className={styles.dotsContainer}>
          <Box className={styles.dot} />
          <Box className={styles.dot} />
          <Box className={styles.dot} />
        </Box>

        <Flex direction="column" gap="3" className={styles.skeletonContainer}>
          <Box className={styles.skeletonLine} style={{ width: "85%" }} />
          <Box className={styles.skeletonLine} style={{ width: "70%" }} />
          <Box className={styles.skeletonLine} style={{ width: "90%" }} />
          <Box className={styles.skeletonLine} style={{ width: "60%" }} />
        </Flex>
      </Flex>
    </Container>
  );
};

ChatLoading.displayName = "ChatLoading";

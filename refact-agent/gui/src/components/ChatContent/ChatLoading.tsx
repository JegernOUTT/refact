import React from "react";
import { Flex, Container, Box } from "@radix-ui/themes";
import styles from "./ChatLoading.module.css";

const skeletonWidths = ["85%", "70%", "90%", "60%"];

export const ChatLoading: React.FC = () => {
  return (
    <Container>
      <Flex
        direction="column"
        align="center"
        justify="center"
        gap="4"
        py="9"
        className={`${styles.container} rf-enter-rise`}
      >
        <Box className={styles.dotsContainer}>
          <Box className={styles.dot} />
          <Box className={styles.dot} />
          <Box className={styles.dot} />
        </Box>

        <Flex direction="column" gap="3" className={styles.skeletonContainer}>
          {skeletonWidths.map((width) => (
            <Box
              key={width}
              className={styles.skeletonLine}
              style={{ width }}
            />
          ))}
        </Flex>
      </Flex>
    </Container>
  );
};

ChatLoading.displayName = "ChatLoading";

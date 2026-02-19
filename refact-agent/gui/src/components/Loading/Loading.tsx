import React from "react";
import { Flex, Box } from "@radix-ui/themes";
import styles from "./Loading.module.css";

export const Loading: React.FC = () => {
  return (
    <Flex direction="column" gap="2" className={styles.container}>
      <Flex gap="2" align="center">
        <Box className={styles.dot} />
        <Box className={styles.dot} />
        <Box className={styles.dot} />
      </Flex>
      <Box className={styles.skeletonLine} style={{ width: "80%" }} />
      <Box className={styles.skeletonLine} style={{ width: "60%" }} />
    </Flex>
  );
};

Loading.displayName = "Loading";

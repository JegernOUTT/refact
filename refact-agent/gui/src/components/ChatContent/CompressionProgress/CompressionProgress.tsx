import React from "react";
import { Flex, Text } from "@radix-ui/themes";
import { LogoAnimation } from "../../LogoAnimation/LogoAnimation";
import styles from "./CompressionProgress.module.css";

export const CompressionProgress: React.FC = () => (
  <Flex
    align="center"
    gap="2"
    className={styles.card}
    role="status"
    aria-live="polite"
    aria-label="Compressing context"
    data-testid="compression-progress"
  >
    <LogoAnimation size="4" isStreaming={true} isWaiting={false} />
    <Text size="2" color="gray">
      Compressing context…
    </Text>
  </Flex>
);

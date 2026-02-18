import React from "react";
import { Card, Flex, Text } from "@radix-ui/themes";
import styles from "./StatCard.module.css";

export type StatCardProps = {
  title: string;
  value: string;
  subtitle?: string;
};

export const StatCard: React.FC<StatCardProps> = ({ title, value, subtitle }) => (
  <Card className={styles.card}>
    <Flex direction="column">
      <Text size="2" className={styles.title}>
        {title}
      </Text>
      <Text size="7" weight="bold" className={styles.value}>
        {value}
      </Text>
      {subtitle && (
        <Text size="1" className={styles.subtitle}>
          {subtitle}
        </Text>
      )}
    </Flex>
  </Card>
);

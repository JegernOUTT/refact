import React from "react";
import { Flex, Text, Badge } from "@radix-ui/themes";
import type { CompletionDetail } from "../../services/refact/commands";
import styles from "./SlashCommandSuggestion.module.css";

type SlashCommandSuggestionProps = {
  name: string;
  detail?: CompletionDetail;
};

export const SlashCommandSuggestion: React.FC<SlashCommandSuggestionProps> = ({
  name,
  detail,
}) => (
  <Flex direction="row" align="center" gap="2" className={styles.suggestion}>
    <Text weight="bold" size="2" className={styles.name}>
      {name}
    </Text>
    {detail?.description && (
      <Text size="1" color="gray" className={styles.description}>
        {detail.description}
      </Text>
    )}
    <Badge size="1" variant="soft" className={styles.badge}>
      {detail?.kind === "skill" ? "skill" : "cmd"}
    </Badge>
  </Flex>
);

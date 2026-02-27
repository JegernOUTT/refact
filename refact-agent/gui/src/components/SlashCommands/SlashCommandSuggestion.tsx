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
  <Flex direction="column" className={styles.suggestion}>
    <Text weight="bold" size="2" className={styles.name}>
      {name}
    </Text>
    {detail?.description && (
      <Text size="1" color="gray" className={styles.description}>
        {detail.description}
      </Text>
    )}
    {(detail?.argument_hint ?? detail?.source) && (
      <Flex gap="2" align="center">
        {detail?.argument_hint && (
          <Text size="1" className={styles.hint}>
            {detail.argument_hint}
          </Text>
        )}
        {detail?.source && (
          <Badge size="1" variant="soft">
            {detail.source}
          </Badge>
        )}
      </Flex>
    )}
  </Flex>
);

import React from "react";
import { Info } from "lucide-react";
import { Flex, Icon, Surface, Text } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import { selectBuddySnapshot } from "./buddySlice";
import type { BuddyDraft } from "./types";
import styles from "./BuddyDraftPreview.module.css";

type Props = {
  draft: BuddyDraft;
};

export const BuddyDraftPreview: React.FC<Props> = ({ draft }) => {
  const name = useAppSelector(selectBuddySnapshot)?.state.identity.name ?? "";
  const titlePrefix = name ? `${name} draft` : "Draft";

  return (
    <Surface variant="surface-1" animated="rise" className={styles.panel}>
      <Flex align="start" gap="2">
        <Icon icon={Info} size="md" tone="accent" />
        <Flex direction="column" gap="1">
          <Text size="2" weight="bold">
            {titlePrefix}: {draft.title}
          </Text>
          {draft.explanation && <Text size="1">{draft.explanation}</Text>}
        </Flex>
      </Flex>
    </Surface>
  );
};

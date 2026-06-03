import React from "react";
import { Box, Button, Dialog, Flex, Text } from "@radix-ui/themes";
import type { PlanHistoryItem } from "../../../features/Chat/Thread/selectors";
import { getPlanMetadata, isPlanMessage } from "../../../services/refact/types";
import { Markdown } from "../../Markdown";
import styles from "./PlanBanner.module.css";
import { normalizePlanContent } from "./planContent";

type PlanHistoryModalProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  items: PlanHistoryItem[];
};

export const PlanHistoryModal: React.FC<PlanHistoryModalProps> = ({
  open,
  onOpenChange,
  items,
}) => {
  const itemTitle = (item: PlanHistoryItem, index: number): string => {
    if (!isPlanMessage(item)) return `📋 Plan update ${index}`;

    const label = index === 0 ? "Base plan" : "Plan";
    const metadata = getPlanMetadata(item);
    const mode = metadata.mode ?? "Mode unknown";
    const version =
      metadata.version !== undefined ? `v${metadata.version}` : "v?";
    return `📋 ${label} — ${mode} · ${version}`;
  };

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Content className={styles.modalContent}>
        <Dialog.Title>Plan history</Dialog.Title>
        <Dialog.Description size="2" color="gray">
          Base plan and append-only updates for this chat.
        </Dialog.Description>

        <Flex direction="column" gap="3" mt="3" className={styles.historyList}>
          {items.map((item, index) => (
            <Box
              key={item.message_id ?? `${index}-${item.content}`}
              className={styles.historyItem}
            >
              <Text
                as="div"
                size="2"
                weight="bold"
                className={styles.historyTitle}
              >
                {itemTitle(item, index)}
              </Text>
              <Box className={styles.historyBody}>
                <Markdown>{normalizePlanContent(item.content)}</Markdown>
              </Box>
            </Box>
          ))}
        </Flex>

        <Flex justify="end" mt="4">
          <Dialog.Close>
            <Button variant="soft" color="gray">
              Close
            </Button>
          </Dialog.Close>
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};

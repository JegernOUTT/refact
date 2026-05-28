import React from "react";
import { Box, Button, Dialog, Flex, Text } from "@radix-ui/themes";
import {
  getPlanMetadata,
  type PlanMessage,
} from "../../../services/refact/types";
import { Markdown } from "../../Markdown";
import styles from "./PlanBanner.module.css";

type PlanHistoryModalProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  plans: PlanMessage[];
};

export const PlanHistoryModal: React.FC<PlanHistoryModalProps> = ({
  open,
  onOpenChange,
  plans,
}) => {
  const planTitle = (plan: PlanMessage): string => {
    const metadata = getPlanMetadata(plan);
    const mode = metadata.mode ?? "Mode unknown";
    const version =
      metadata.version !== undefined ? `v${metadata.version}` : "v?";
    return `📋 Plan — ${mode} · ${version}`;
  };

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Content className={styles.modalContent}>
        <Dialog.Title>Plan history</Dialog.Title>
        <Dialog.Description size="2" color="gray">
          Previous plan versions for this chat.
        </Dialog.Description>

        <Flex direction="column" gap="3" mt="3" className={styles.historyList}>
          {plans.map((plan, index) => (
            <Box
              key={plan.message_id ?? `${index}-${plan.content}`}
              className={styles.historyItem}
            >
              <Text
                as="div"
                size="2"
                weight="bold"
                className={styles.historyTitle}
              >
                {planTitle(plan)}
              </Text>
              <Box className={styles.historyBody}>
                <Markdown>{plan.content}</Markdown>
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

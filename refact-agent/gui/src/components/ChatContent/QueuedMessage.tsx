import React, { useCallback } from "react";
import { Flex, Text, IconButton, Card, Badge } from "@radix-ui/themes";
import {
  Cross1Icon,
  ClockIcon,
  LightningBoltIcon,
} from "@radix-ui/react-icons";
import { QueuedItem } from "../../features/Chat";
import { useChatActions } from "../../hooks";
import styles from "./ChatContent.module.css";
import classNames from "classnames";

type QueuedMessageProps = {
  queuedItem: QueuedItem;
  position: number;
};

export const QueuedMessage: React.FC<QueuedMessageProps> = ({
  queuedItem,
  position,
}) => {
  const { cancelQueued } = useChatActions();

  const handleCancel = useCallback(() => {
    void cancelQueued(queuedItem.client_request_id);
  }, [cancelQueued, queuedItem.client_request_id]);

  return (
    <Card
      className={classNames(styles.queuedMessage, {
        [styles.queuedMessagePriority]: queuedItem.priority,
      })}
    >
      <Flex gap="2" align="center" justify="between">
        <Flex gap="2" align="center" style={{ flex: 1, minWidth: 0 }}>
          <Badge
            color={queuedItem.priority ? "blue" : "amber"}
            variant="soft"
            size="1"
          >
            {queuedItem.priority ? (
              <LightningBoltIcon width={12} height={12} />
            ) : (
              <ClockIcon width={12} height={12} />
            )}
            {position}
          </Badge>
          <Text
            size="2"
            color="gray"
            className={styles.queuedMessageText}
            title={queuedItem.preview}
          >
            {queuedItem.preview || `[${queuedItem.command_type}]`}
          </Text>
        </Flex>
        <IconButton
          size="1"
          variant="ghost"
          color="gray"
          onClick={handleCancel}
          title="Cancel queued message"
        >
          <Cross1Icon width={14} height={14} />
        </IconButton>
      </Flex>
    </Card>
  );
};

export default QueuedMessage;

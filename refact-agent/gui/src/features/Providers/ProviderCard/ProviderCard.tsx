import React from "react";
import { Badge, Card, Flex, Heading } from "@radix-ui/themes";

import { iconsMap } from "../icons/iconsMap";

import type {
  ProviderListItem,
  ProviderStatus,
} from "../../../services/refact";

import { getProviderName } from "../getProviderName";

import styles from "./ProviderCard.module.css";

export type ProviderCardProps = {
  provider: ProviderListItem;
  setCurrentProvider: (provider: ProviderListItem) => void;
};

const StatusDot: React.FC<{ status: ProviderStatus }> = ({ status }) => {
  switch (status) {
    case "active":
      return (
        <Badge color="green" size="1" variant="soft">
          ●
        </Badge>
      );
    case "configured":
      return (
        <Badge color="orange" size="1" variant="soft">
          ●
        </Badge>
      );
    default:
      return null;
  }
};

export const ProviderCard: React.FC<ProviderCardProps> = ({
  provider,
  setCurrentProvider,
}) => {
  return (
    <Card
      size="2"
      onClick={() => setCurrentProvider(provider)}
      className={styles.providerCard}
    >
      <Flex align="center" justify="between">
        <Flex gap="3" align="center">
          {iconsMap[provider.name]}
          <Heading as="h6" size="2">
            {getProviderName(provider)}
          </Heading>
        </Flex>
        <Flex align="center" gap="2">
          {provider.model_count > 0 && (
            <Badge color="gray" size="1" variant="soft">
              {provider.model_count} model
              {provider.model_count !== 1 ? "s" : ""}
            </Badge>
          )}
          <StatusDot status={provider.status} />
        </Flex>
      </Flex>
    </Card>
  );
};

import React from "react";

import { Flex, Heading, Text } from "@radix-ui/themes";
import { ProviderCard } from "../ProviderCard/ProviderCard";

import type { ProviderListItem } from "../../../services/refact";
import { useGetConfiguredProvidersView } from "./useConfiguredProvidersView";

export type ConfiguredProvidersViewProps = {
  configuredProviders: ProviderListItem[];
  handleSetCurrentProvider: (provider: ProviderListItem) => void;
};

export const ConfiguredProvidersView: React.FC<
  ConfiguredProvidersViewProps
> = ({ configuredProviders, handleSetCurrentProvider }) => {
  const { sortedConfiguredProviders } = useGetConfiguredProvidersView({
    configuredProviders,
  });

  return (
    <Flex direction="column" gap="2" justify="between" height="100%">
      <Flex direction="column" gap="2">
        <Flex direction="column" gap="1">
          <Heading as="h2" size="3">
            Configured Providers
          </Heading>
          <Text as="p" size="2" color="gray">
            Here you can navigate through the list of configured and available
            providers
          </Text>
        </Flex>
        {sortedConfiguredProviders.map((provider, idx) => (
          <ProviderCard
            key={`${provider.name}_${idx}`}
            provider={provider}
            setCurrentProvider={handleSetCurrentProvider}
          />
        ))}
      </Flex>
    </Flex>
  );
};

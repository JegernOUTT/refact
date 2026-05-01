import React, { useCallback } from "react";
import { Box, Button, Card, Flex, Grid, Heading, Text } from "@radix-ui/themes";
import { ScrollArea } from "../../components/ScrollArea";
import { useAppDispatch, useGetConfiguredProvidersQuery } from "../../hooks";
import { ProviderCard } from "../Providers/ProviderCard";
import { ProviderPreview } from "../Providers/ProviderPreview";
import type { ProviderListItem } from "../../services/refact";
import { useGetConfiguredProvidersView } from "../Providers/ProvidersView/useConfiguredProvidersView";
import { push } from "../Pages/pagesSlice";
import { getProviderName } from "../Providers/getProviderName";
import { hasAnyUsableActiveProvider } from "./providerAccess";

export const LoginPage: React.FC = () => {
  const dispatch = useAppDispatch();
  const providersQuery = useGetConfiguredProvidersQuery();
  const configuredProviders = providersQuery.data?.providers ?? [];
  const { sortedConfiguredProviders } = useGetConfiguredProvidersView({
    configuredProviders,
  });
  const [currentProviderName, setCurrentProviderName] = React.useState<
    string | null
  >(null);
  const currentProvider = React.useMemo(() => {
    return (
      sortedConfiguredProviders.find(
        (provider) => provider.name === currentProviderName,
      ) ?? null
    );
  }, [currentProviderName, sortedConfiguredProviders]);

  const hasAnyActiveProvider = React.useMemo(() => {
    return hasAnyUsableActiveProvider({
      providers: sortedConfiguredProviders,
    });
  }, [sortedConfiguredProviders]);

  const providerStatusLabel = React.useMemo(() => {
    if (providersQuery.isFetching || providersQuery.isLoading) {
      return "Loading providers…";
    }
    if (providersQuery.isUninitialized) {
      return "Connecting to backend…";
    }
    if (providersQuery.isError) {
      return "Unable to load providers";
    }
    if (hasAnyActiveProvider) {
      return "Ready to start";
    }
    return "Enable at least one model to continue";
  }, [
    hasAnyActiveProvider,
    providersQuery.isError,
    providersQuery.isFetching,
    providersQuery.isLoading,
    providersQuery.isUninitialized,
  ]);

  const onContinue = useCallback(() => {
    dispatch(push({ name: "history" }));
  }, [dispatch]);

  return (
    <ScrollArea scrollbars="vertical" fullHeight>
      <Box mx="auto" p="6" style={{ maxWidth: 960 }}>
        <Flex direction="column" gap="4">
          <Heading align="center" as="h2" size="6">
            Set Up Providers
          </Heading>
          <Text size="2" color="gray" align="center">
            Configure at least one BYOK provider or local runtime, enable a
            model, then continue.
          </Text>

          {!currentProvider && (
            <>
              <Grid columns={{ initial: "2", sm: "3" }} gap="3" width="100%">
                {sortedConfiguredProviders.map((provider) => (
                  <ProviderCard
                    key={provider.name}
                    provider={provider}
                    setCurrentProvider={() =>
                      setCurrentProviderName(provider.name)
                    }
                  />
                ))}
              </Grid>
              {providersQuery.isError && (
                <Card variant="surface">
                  <Text size="2" color="gray">
                    Unable to load providers from the backend. Check that the
                    local Refact engine is running and the UI is using the
                    correct port.
                  </Text>
                </Card>
              )}
              {!providersQuery.isSuccess && !providersQuery.isError && (
                <Card variant="surface">
                  <Text size="2" color="gray">
                    Waiting for the local Refact engine before loading
                    providers.
                  </Text>
                </Card>
              )}
              {providersQuery.isSuccess &&
                sortedConfiguredProviders.length === 0 && (
                  <Card variant="surface">
                    <Text size="2" color="gray">
                      The backend returned an empty provider list. Restart the
                      local Refact engine, then open the Providers screen again.
                    </Text>
                  </Card>
                )}
            </>
          )}

          {currentProvider && (
            <Card variant="surface" style={{ padding: "var(--space-4)" }}>
              <Flex justify="between" align="center" mb="3" gap="3" wrap="wrap">
                <Heading as="h4" size="3">
                  {getProviderName(currentProvider)}
                </Heading>
                <Button
                  variant="outline"
                  onClick={() => setCurrentProviderName(null)}
                >
                  Back to providers
                </Button>
              </Flex>
              <ProviderPreview
                configuredProviders={sortedConfiguredProviders}
                currentProvider={currentProvider}
                handleSetCurrentProvider={(provider: ProviderListItem | null) =>
                  setCurrentProviderName(provider?.name ?? null)
                }
              />
            </Card>
          )}

          <Flex justify="end" gap="3" mt="2" align="center" wrap="wrap">
            <Text size="2" color="gray">
              {providerStatusLabel}
            </Text>
            <Button
              onClick={onContinue}
              disabled={
                !providersQuery.isSuccess ||
                providersQuery.isFetching ||
                !hasAnyActiveProvider
              }
            >
              Continue
            </Button>
          </Flex>
        </Flex>
      </Box>
    </ScrollArea>
  );
};

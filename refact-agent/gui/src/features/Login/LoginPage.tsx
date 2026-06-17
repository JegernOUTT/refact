import React, { useCallback } from "react";
import classNames from "classnames";
import { ArrowLeft } from "lucide-react";
import { ScrollArea } from "../../components/ScrollArea";
import { Button, EmptyState, LoadingState } from "../../components/ui";
import { useAppDispatch, useProviderBootstrapState } from "../../hooks";
import { ProviderCard } from "../Providers/ProviderCard";
import { ProviderPreview } from "../Providers/ProviderPreview";
import type { ProviderListItem } from "../../services/refact";
import { useGetConfiguredProvidersView } from "../Providers/ProvidersView/useConfiguredProvidersView";
import { push } from "../Pages/pagesSlice";
import { getProviderName } from "../Providers/getProviderName";
import styles from "./LoginPage.module.css";

export const LoginPage: React.FC = () => {
  const dispatch = useAppDispatch();
  const providerBootstrap = useProviderBootstrapState();
  const providersQuery = providerBootstrap.providersQuery;
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

  const hasAnyActiveProvider = providerBootstrap.hasAnyActiveProvider;
  const canShowProviderSetup = providerBootstrap.canShowProviderSetup;
  const visibleCurrentProvider = canShowProviderSetup ? currentProvider : null;

  const bootstrapCopy = React.useMemo(() => {
    switch (providerBootstrap.status) {
      case "backend_connecting":
        return {
          title: "Connecting to Refact",
          subtitle:
            "Waiting for the local Refact engine before loading providers.",
          status: "Connecting to backend…",
        };
      case "backend_offline":
        return {
          title: "Connection Problem",
          subtitle: "The local Refact engine is not reachable yet.",
          status: "Backend server unreachable",
        };
      case "provider_loading":
        return {
          title: "Loading Providers",
          subtitle: "Refact is checking provider and model availability.",
          status: "Loading providers…",
        };
      case "provider_error":
        return {
          title: "Unable to Load Providers",
          subtitle:
            "Check that the local Refact engine is running and the UI is using the correct port.",
          status: "Unable to load providers",
        };
      case "ready":
        return {
          title: "Providers Ready",
          subtitle: "At least one provider is active and ready for chat.",
          status: "Ready to start",
        };
      case "setup_required":
        return {
          title: "Set Up Providers",
          subtitle:
            "Configure at least one BYOK provider or local runtime, enable a model, then continue.",
          status: "Enable at least one model to continue",
        };
    }
  }, [providerBootstrap.status]);

  const onContinue = useCallback(() => {
    dispatch(push({ name: "history" }));
  }, [dispatch]);

  return (
    <ScrollArea scrollbars="vertical" fullHeight>
      <main className={classNames(styles.page, "rf-enter")}>
        <section className={styles.hero}>
          <p className={styles.kicker}>Welcome to Refact</p>
          <h2 className={styles.title}>{bootstrapCopy.title}</h2>
          <p className={styles.subtitle}>{bootstrapCopy.subtitle}</p>
        </section>

        {!visibleCurrentProvider && canShowProviderSetup && (
          <>
            <div className={classNames(styles.providerGrid, "rf-stagger")}>
              {sortedConfiguredProviders.map((provider) => (
                <ProviderCard
                  key={provider.name}
                  provider={provider}
                  setCurrentProvider={() =>
                    setCurrentProviderName(provider.name)
                  }
                />
              ))}
            </div>
            {sortedConfiguredProviders.length === 0 && (
              <EmptyState
                title="No providers found"
                description="Restart the local Refact engine, then open the Providers screen again."
              />
            )}
          </>
        )}

        {!visibleCurrentProvider && !canShowProviderSetup && (
          <div className={styles.bootstrapState}>
            {providerBootstrap.status === "provider_error" ? (
              <EmptyState
                title="Unable to load providers"
                description="Check that the local Refact engine is running and the UI is using the correct port."
              />
            ) : (
              <LoadingState label={bootstrapCopy.status} variant="full" />
            )}
          </div>
        )}

        {visibleCurrentProvider && (
          <section
            className={classNames(styles.providerPreview, "rf-enter-rise")}
          >
            <div className={styles.providerHeader}>
              <h3 className={styles.providerTitle}>
                {getProviderName(visibleCurrentProvider)}
              </h3>
              <Button
                variant="soft"
                leftIcon={ArrowLeft}
                onClick={() => setCurrentProviderName(null)}
              >
                Back to providers
              </Button>
            </div>
            <ProviderPreview
              configuredProviders={sortedConfiguredProviders}
              currentProvider={visibleCurrentProvider}
              handleSetCurrentProvider={(provider: ProviderListItem | null) =>
                setCurrentProviderName(provider?.name ?? null)
              }
            />
          </section>
        )}

        <footer className={styles.footer}>
          <span className={styles.status}>{bootstrapCopy.status}</span>
          <Button
            variant="primary"
            onClick={onContinue}
            disabled={
              !providersQuery.isSuccess ||
              providersQuery.isFetching ||
              !hasAnyActiveProvider
            }
          >
            Continue
          </Button>
        </footer>
      </main>
    </ScrollArea>
  );
};

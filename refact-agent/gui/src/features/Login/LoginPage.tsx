import React, { useCallback } from "react";
import classNames from "classnames";
import { ArrowLeft } from "lucide-react";
import { ScrollArea } from "../../components/ScrollArea";
import { Button, EmptyState, LoadingState } from "../../components/ui";
import { useAppDispatch, useGetConfiguredProvidersQuery } from "../../hooks";
import { ProviderCard } from "../Providers/ProviderCard";
import { ProviderPreview } from "../Providers/ProviderPreview";
import type { ProviderListItem } from "../../services/refact";
import { useGetConfiguredProvidersView } from "../Providers/ProvidersView/useConfiguredProvidersView";
import { push } from "../Pages/pagesSlice";
import { getProviderName } from "../Providers/getProviderName";
import { hasAnyUsableActiveProvider } from "./providerAccess";
import styles from "./LoginPage.module.css";

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
      <main className={classNames(styles.page, "rf-enter")}>
        <section className={styles.hero}>
          <p className={styles.kicker}>Welcome to Refact</p>
          <h2 className={styles.title}>Set Up Providers</h2>
          <p className={styles.subtitle}>
            Configure at least one BYOK provider or local runtime, enable a
            model, then continue.
          </p>
        </section>

        {!currentProvider && (
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
            {providersQuery.isError && (
              <EmptyState
                title="Unable to load providers"
                description="Check that the local Refact engine is running and the UI is using the correct port."
              />
            )}
            {!providersQuery.isSuccess && !providersQuery.isError && (
              <LoadingState label="Waiting for the local Refact engine before loading providers." />
            )}
            {providersQuery.isSuccess &&
              sortedConfiguredProviders.length === 0 && (
                <EmptyState
                  title="No providers found"
                  description="Restart the local Refact engine, then open the Providers screen again."
                />
              )}
          </>
        )}

        {currentProvider && (
          <section
            className={classNames(styles.providerPreview, "rf-enter-rise")}
          >
            <div className={styles.providerHeader}>
              <h3 className={styles.providerTitle}>
                {getProviderName(currentProvider)}
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
              currentProvider={currentProvider}
              handleSetCurrentProvider={(provider: ProviderListItem | null) =>
                setCurrentProviderName(provider?.name ?? null)
              }
            />
          </section>
        )}

        <footer className={styles.footer}>
          <span className={styles.status}>{providerStatusLabel}</span>
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

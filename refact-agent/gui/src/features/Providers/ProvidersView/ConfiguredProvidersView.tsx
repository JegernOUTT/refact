import React from "react";
import { Plus } from "lucide-react";

import { Button, EmptyState } from "../../../components/ui";
import { ProviderCard } from "../ProviderCard/ProviderCard";

import type { ProviderListItem } from "../../../services/refact";
import { useGetConfiguredProvidersView } from "./useConfiguredProvidersView";
import styles from "./ProvidersView.module.css";

export type ConfiguredProvidersViewProps = {
  configuredProviders: ProviderListItem[];
  handleSetCurrentProvider: (provider: ProviderListItem) => void;
  onAddInstance: () => void;
  onDuplicateProvider: (provider: ProviderListItem) => void;
};

export const ConfiguredProvidersView: React.FC<ConfiguredProvidersViewProps> = ({
  configuredProviders,
  handleSetCurrentProvider,
  onAddInstance,
  onDuplicateProvider,
}) => {
  const { sortedConfiguredProviders } = useGetConfiguredProvidersView({ configuredProviders });

  return (
    <section className={styles.configuredView}>
      <div className={styles.headerRow}>
        <div className={styles.headerCopy}>
          <h2 className={styles.title}>Configured Providers</h2>
          <p className={styles.description}>
            Here you can navigate through the list of configured and available providers
          </p>
        </div>
        <Button variant="soft" size="md" leftIcon={Plus} onClick={onAddInstance}>
          Add instance
        </Button>
      </div>
      {sortedConfiguredProviders.length > 0 ? (
        <div className="rf-stagger">
          <div className={styles.providersGrid}>
            {sortedConfiguredProviders.map((provider, idx) => (
              <div className="rf-enter" key={`${provider.name}_${idx}`}>
                <ProviderCard
                  provider={provider}
                  setCurrentProvider={handleSetCurrentProvider}
                  onDuplicateProvider={onDuplicateProvider}
                />
              </div>
            ))}
          </div>
        </div>
      ) : (
        <EmptyState
          title="No providers configured"
          description="Add a provider instance to start using models in chat."
          action={
            <Button variant="primary" leftIcon={Plus} onClick={onAddInstance}>
              Add instance
            </Button>
          }
        />
      )}
    </section>
  );
};

import React from "react";
import { Plus } from "lucide-react";

import { Button, EmptyState } from "../../../components/ui";
import { SettingsGroup } from "../../Settings/SettingsSection";
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

export const ConfiguredProvidersView: React.FC<
  ConfiguredProvidersViewProps
> = ({
  configuredProviders,
  handleSetCurrentProvider,
  onAddInstance,
  onDuplicateProvider,
}) => {
  const { sortedConfiguredProviders } = useGetConfiguredProvidersView({
    configuredProviders,
  });

  return (
    <SettingsGroup title="Configured providers">
      {sortedConfiguredProviders.length > 0 ? (
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
      ) : (
        <EmptyState
          variant="full"
          title="No providers configured"
          description="Add a provider instance to start using models in chat."
          icon={Plus}
          action={
            <Button variant="primary" leftIcon={Plus} onClick={onAddInstance}>
              Add instance
            </Button>
          }
        />
      )}
    </SettingsGroup>
  );
};

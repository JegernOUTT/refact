import React, { useState, useMemo, useCallback } from "react";
import { ChevronDown, ChevronRight, Plus } from "lucide-react";
import { useDebounceCallback } from "usehooks-ts";
import {
  useGetMarketplacesQuery,
  useGetMarketplacePluginsQuery,
  useGetInstalledQuery,
  useDeleteMarketplaceMutation,
  useUninstallPluginMutation,
} from "../../../services/refact/plugins";
import type {
  MarketplaceEntry,
  PluginEntry,
} from "../../../services/refact/plugins";
import {
  Button,
  EmptyState,
  FieldError,
  FieldText,
  Icon,
} from "../../../components/ui";
import { Spinner } from "../../../components/Spinner";
import { AddMarketplaceDialog } from "./AddMarketplaceDialog";
import { MarketplacePluginCard } from "./MarketplacePluginCard";

import styles from "./MarketplacePanel.module.css";

type MarketplaceSectionProps = {
  marketplace: MarketplaceEntry;
  searchQuery: string;
  installedIds: Set<string>;
};

const MarketplaceSection: React.FC<MarketplaceSectionProps> = ({
  marketplace,
  searchQuery,
  installedIds,
}) => {
  const { data, isLoading, isError } = useGetMarketplacePluginsQuery(
    marketplace.name,
  );
  const [deleteMarketplace, { isLoading: deleting }] =
    useDeleteMarketplaceMutation();

  const handleDelete = useCallback(() => {
    void deleteMarketplace(marketplace.name);
  }, [deleteMarketplace, marketplace.name]);

  const filteredPlugins = useMemo<PluginEntry[]>(() => {
    if (!data) return [];
    if (!searchQuery) return data.plugins;
    const q = searchQuery.toLowerCase();
    return data.plugins.filter(
      (p) =>
        p.name.toLowerCase().includes(q) ||
        p.description.toLowerCase().includes(q),
    );
  }, [data, searchQuery]);

  return (
    <section className={`${styles.marketplaceSection} rf-enter`}>
      <div className={styles.marketplaceHeader}>
        <div className={styles.marketplaceTitle}>
          <h3 className={styles.heading}>{marketplace.name}</h3>
          <span className={styles.muted}>{marketplace.source}</span>
          {data && (
            <span className={styles.muted}>
              ({data.plugins.length} plugins)
            </span>
          )}
        </div>
        <Button
          size="sm"
          variant="danger"
          onClick={handleDelete}
          disabled={deleting}
          loading={deleting}
        >
          Remove
        </Button>
      </div>

      {isLoading && (
        <div className={styles.marketplaceTitle}>
          <Spinner spinning />
          <span className={styles.muted}>Loading plugins…</span>
        </div>
      )}

      {isError && (
        <FieldError>Failed to load plugins for this marketplace.</FieldError>
      )}

      {!isLoading && !isError && filteredPlugins.length === 0 && (
        <EmptyState
          title={
            searchQuery ? "No plugins match your search" : "No plugins found"
          }
        />
      )}

      {filteredPlugins.length > 0 && (
        <div className={`${styles.pluginsGrid} rf-stagger`}>
          {filteredPlugins.map((plugin) => (
            <MarketplacePluginCard
              key={plugin.name}
              plugin={plugin}
              isInstalled={installedIds.has(plugin.name)}
            />
          ))}
        </div>
      )}
    </section>
  );
};

export const MarketplacePanel: React.FC = () => {
  const [dialogOpen, setDialogOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [installedExpanded, setInstalledExpanded] = useState(true);

  const debouncedSetSearch = useDebounceCallback(setDebouncedSearch, 300);

  const handleSearchChange = useCallback(
    (value: string) => {
      setSearch(value);
      debouncedSetSearch(value);
    },
    [debouncedSetSearch],
  );

  const {
    data: marketplacesData,
    isLoading: loadingMarketplaces,
    isError: marketplacesError,
    refetch,
  } = useGetMarketplacesQuery(undefined);
  const { data: installedData } = useGetInstalledQuery(undefined);
  const [uninstallPlugin] = useUninstallPluginMutation();

  const installedIds = useMemo<Set<string>>(() => {
    if (!installedData) return new Set();
    return new Set(installedData.installed.map((p) => p.name));
  }, [installedData]);

  const marketplaces = marketplacesData?.marketplaces ?? [];
  const installed = installedData?.installed ?? [];

  if (!loadingMarketplaces && marketplacesError) {
    return (
      <div className={styles.panel}>
        <EmptyState
          action={
            <Button size="sm" variant="soft" onClick={() => void refetch()}>
              Retry
            </Button>
          }
          title="Failed to load marketplaces"
          variant="full"
        />
      </div>
    );
  }

  if (!loadingMarketplaces && marketplaces.length === 0) {
    return (
      <div className={styles.panel}>
        <EmptyState
          className={styles.onboarding}
          title="Plugin Marketplace"
          description={
            <>
              Add a marketplace source to discover and install plugins. A
              marketplace is a Git repository containing plugin definitions.
              <br />
              Example: JegernOUTT/refact-plugins
            </>
          }
          action={
            <Button
              variant="primary"
              onClick={() => setDialogOpen(true)}
              leftIcon={Plus}
            >
              Add Marketplace
            </Button>
          }
          variant="full"
        />

        <AddMarketplaceDialog
          open={dialogOpen}
          onClose={() => setDialogOpen(false)}
        />
      </div>
    );
  }

  return (
    <div className={`${styles.panel} rf-stagger`}>
      {loadingMarketplaces && (
        <div className={styles.marketplaceTitle}>
          <Spinner spinning />
          <span className={styles.muted}>Loading marketplaces…</span>
        </div>
      )}

      <div className={styles.toolbar}>
        <Button
          variant="primary"
          onClick={() => setDialogOpen(true)}
          leftIcon={Plus}
        >
          Add Marketplace
        </Button>
        <div className={styles.searchInput}>
          <FieldText
            placeholder="Search plugins…"
            value={search}
            onChange={handleSearchChange}
          />
        </div>
      </div>

      {installed.length > 0 && (
        <section className={styles.installedSection}>
          <button
            className={`${styles.installedHeader} rf-pressable`}
            type="button"
            aria-label="Toggle installed plugins"
            onClick={() => setInstalledExpanded((v) => !v)}
          >
            <Icon
              icon={installedExpanded ? ChevronDown : ChevronRight}
              size="sm"
            />
            <h3 className={styles.heading}>Installed ({installed.length})</h3>
          </button>
          {installedExpanded && (
            <div className={`${styles.installedList} rf-stagger`}>
              {installed.map((plugin) => (
                <div key={plugin.name} className={styles.installedItem}>
                  <div className={styles.installedInfo}>
                    <span className={styles.heading}>{plugin.name}</span>
                    <span className={styles.muted}>
                      Installed{" "}
                      {new Date(plugin.installed_at).toLocaleDateString()}
                    </span>
                  </div>
                  <Button
                    size="sm"
                    variant="danger"
                    onClick={() => void uninstallPlugin(plugin.name)}
                  >
                    Uninstall
                  </Button>
                </div>
              ))}
            </div>
          )}
        </section>
      )}

      {marketplaces.map((marketplace) => (
        <MarketplaceSection
          key={marketplace.name}
          marketplace={marketplace}
          searchQuery={debouncedSearch}
          installedIds={installedIds}
        />
      ))}

      <AddMarketplaceDialog
        open={dialogOpen}
        onClose={() => setDialogOpen(false)}
      />
    </div>
  );
};

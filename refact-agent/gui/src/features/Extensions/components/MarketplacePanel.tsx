import React, { useState, useMemo, useCallback } from "react";
import { Button, Flex, Spinner, Text, TextField } from "@radix-ui/themes";
import { ChevronDownIcon, ChevronRightIcon } from "@radix-ui/react-icons";
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
    <div className={styles.marketplaceSection}>
      <div className={styles.marketplaceHeader}>
        <Flex align="center" gap="2">
          <Text size="2" weight="bold">
            {marketplace.name}
          </Text>
          <Text size="1" color="gray">
            {marketplace.source}
          </Text>
          {data && (
            <Text size="1" color="gray">
              ({data.plugins.length} plugins)
            </Text>
          )}
        </Flex>
        <Button
          size="1"
          variant="ghost"
          color="red"
          onClick={handleDelete}
          disabled={deleting}
        >
          {deleting ? <Spinner size="1" /> : "Remove"}
        </Button>
      </div>

      {isLoading && (
        <Flex align="center" gap="2" py="2">
          <Spinner size="1" />
          <Text size="1" color="gray">
            Loading plugins…
          </Text>
        </Flex>
      )}

      {isError && (
        <Text size="1" color="red">
          Failed to load plugins for this marketplace.
        </Text>
      )}

      {!isLoading && !isError && filteredPlugins.length === 0 && (
        <Text size="1" color="gray">
          {searchQuery ? "No plugins match your search." : "No plugins found."}
        </Text>
      )}

      {filteredPlugins.length > 0 && (
        <div className={styles.pluginsGrid}>
          {filteredPlugins.map((plugin) => (
            <MarketplacePluginCard
              key={plugin.name}
              plugin={plugin}
              isInstalled={installedIds.has(plugin.name)}
            />
          ))}
        </div>
      )}
    </div>
  );
};

export const MarketplacePanel: React.FC = () => {
  const [dialogOpen, setDialogOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [installedExpanded, setInstalledExpanded] = useState(true);

  const debouncedSetSearch = useDebounceCallback(setDebouncedSearch, 300);

  const handleSearchChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      setSearch(e.target.value);
      debouncedSetSearch(e.target.value);
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
        <Flex direction="column" align="center" gap="3" py="6">
          <Text size="2" color="red">
            Failed to load marketplaces.
          </Text>
          <Button size="1" variant="soft" onClick={() => void refetch()}>
            Retry
          </Button>
        </Flex>
      </div>
    );
  }

  if (!loadingMarketplaces && marketplaces.length === 0) {
    return (
      <div className={styles.panel}>
        <Flex
          direction="column"
          align="center"
          gap="3"
          className={styles.onboarding}
        >
          <Text size="3" weight="bold">
            Plugin Marketplace
          </Text>
          <Text size="2" color="gray" align="center">
            Add a marketplace source to discover and install plugins. A
            marketplace is a Git repository containing plugin definitions.
          </Text>
          <Text size="1" color="gray" align="center">
            Example: JegernOUTT/refact-plugins
          </Text>
          <Button size="2" onClick={() => setDialogOpen(true)}>
            + Add Marketplace
          </Button>
        </Flex>

        <AddMarketplaceDialog
          open={dialogOpen}
          onClose={() => setDialogOpen(false)}
        />
      </div>
    );
  }

  return (
    <div className={styles.panel}>
      {loadingMarketplaces && (
        <Flex align="center" gap="2" py="4">
          <Spinner size="2" />
          <Text size="2" color="gray">
            Loading marketplaces…
          </Text>
        </Flex>
      )}

      {installed.length > 0 && (
        <div className={styles.installedSection}>
          <div
            className={styles.installedHeader}
            role="button"
            tabIndex={0}
            aria-label="Toggle installed plugins"
            onClick={() => setInstalledExpanded((v) => !v)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                setInstalledExpanded((v) => !v);
              }
            }}
          >
            {installedExpanded ? (
              <ChevronDownIcon width="14" height="14" />
            ) : (
              <ChevronRightIcon width="14" height="14" />
            )}
            <Text size="2" weight="bold">
              Installed ({installed.length})
            </Text>
          </div>
          {installedExpanded && (
            <div className={styles.installedList}>
              {installed.map((plugin) => (
                <div key={plugin.name} className={styles.installedItem}>
                  <Flex direction="column" gap="1">
                    <Text size="2" weight="medium">
                      {plugin.name}
                    </Text>
                    <Text size="1" color="gray">
                      Installed{" "}
                      {new Date(plugin.installed_at).toLocaleDateString()}
                    </Text>
                  </Flex>
                  <Button
                    size="1"
                    color="red"
                    variant="soft"
                    onClick={() => void uninstallPlugin(plugin.name)}
                  >
                    Uninstall
                  </Button>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {marketplaces.map((marketplace) => (
        <MarketplaceSection
          key={marketplace.name}
          marketplace={marketplace}
          searchQuery={debouncedSearch}
          installedIds={installedIds}
        />
      ))}

      <div className={styles.toolbar}>
        <Button size="2" onClick={() => setDialogOpen(true)}>
          + Add Marketplace
        </Button>
        <div className={styles.searchInput}>
          <TextField.Root
            placeholder="Search plugins…"
            value={search}
            onChange={handleSearchChange}
          />
        </div>
      </div>

      <AddMarketplaceDialog
        open={dialogOpen}
        onClose={() => setDialogOpen(false)}
      />
    </div>
  );
};

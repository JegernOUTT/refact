import React, { useMemo, useState } from "react";
import { ArrowLeft, Info, Search } from "lucide-react";
import { ScrollArea } from "../../components/ScrollArea";
import { PageWrapper } from "../../components/PageWrapper";
import {
  Badge,
  Button,
  EmptyState,
  ErrorState,
  FieldText,
  Icon,
  LoadingState,
  VirtualizedGrid,
} from "../../components/ui";
import {
  useGetMarketplaceQuery,
  useGetInstalledServersQuery,
  useInstallServerMutation,
  useUpdateServerMutation,
  useUninstallServerMutation,
} from "../../services/refact/mcpMarketplace";
import type {
  MCPServer,
  MarketplaceSource,
} from "../../services/refact/mcpMarketplace";
import { ServerCard } from "./ServerCard";
import { ServerDetail } from "./ServerDetail";
import { SourceSelector } from "./SourceSelector";
import { SourceSettings } from "./SourceSettings";
import { requiredEnvKeys } from "./requiredEnv";
import { installErrorMessage, installedKey } from "./installError";
import styles from "./MCPMarketplace.module.css";
import type { Config } from "../Config/configSlice";
import { useAppDispatch } from "../../hooks";
import { integrationsApi } from "../../services/refact/integrations";
import { change } from "../Pages/pagesSlice";

const PAGE_SIZE = 20;

const SERVER_CARD_HEIGHT = 240;

type MCPMarketplaceProps = {
  host: Config["host"];
  tabbed: Config["tabbed"];
  backFromMarketplace: () => void;
  embedded?: boolean;
};

export const MCPMarketplace: React.FC<MCPMarketplaceProps> = ({
  host,
  backFromMarketplace,
  embedded = false,
}) => {
  const dispatch = useAppDispatch();
  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [selectedTag, setSelectedTag] = useState<string | null>(null);
  const [selectedSource, setSelectedSource] = useState<string | null>(null);
  const [selectedServer, setSelectedServer] = useState<MCPServer | null>(null);
  const [installingKey, setInstallingKey] = useState<string | null>(null);
  const [installError, setInstallError] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [page, setPage] = useState(1);

  React.useEffect(() => {
    const timer = setTimeout(() => {
      setDebouncedSearch(search.trim());
      setPage(1);
    }, 300);
    return () => clearTimeout(timer);
  }, [search]);

  const {
    data: marketplaceData,
    isLoading,
    error,
  } = useGetMarketplaceQuery({
    source: selectedSource ?? undefined,
    q: debouncedSearch || undefined,
    tag: selectedTag ?? undefined,
    page,
    page_size: PAGE_SIZE,
  });
  const { data: installedData } = useGetInstalledServersQuery(undefined);
  const [installServer] = useInstallServerMutation();
  const [updateServer] = useUpdateServerMutation();
  const [uninstallServer] = useUninstallServerMutation();

  const sources = useMemo<MarketplaceSource[]>(
    () => marketplaceData?.sources ?? [],
    [marketplaceData?.sources],
  );

  const sourceMap = useMemo(() => {
    const map = new Map<string, string>();
    sources.forEach((s) => map.set(s.id, s.label));
    return map;
  }, [sources]);

  const installedKeys = useMemo(
    () =>
      new Set(
        (installedData?.installed ?? []).map((s) =>
          installedKey(s.source_id, s.id),
        ),
      ),
    [installedData],
  );

  const installedConfigPaths = useMemo(
    () =>
      new Map(
        (installedData?.installed ?? []).map((s) => [
          installedKey(s.source_id, s.id),
          s.config_path,
        ]),
      ),
    [installedData],
  );

  const handleConfigure = (configPath: string) => {
    dispatch(integrationsApi.util.invalidateTags(["INTEGRATIONS"]));
    dispatch(
      change({ name: "integrations page", integrationPath: configPath }),
    );
  };

  const allTags = useMemo(() => {
    // Server-provided catalog spans the full result set; fall back to the
    // current page's tags for older engines.
    if (marketplaceData?.all_tags?.length) return marketplaceData.all_tags;
    const tagSet = new Set<string>();
    (marketplaceData?.servers ?? []).forEach((s) =>
      s.tags.forEach((t) => tagSet.add(t)),
    );
    return Array.from(tagSet).sort();
  }, [marketplaceData]);

  // Search and tag filtering are applied engine-side across all sources.
  const filteredServers = useMemo(
    () => marketplaceData?.servers ?? [],
    [marketplaceData],
  );

  const updateAvailableByKey = useMemo(() => {
    const currentHashes = new Map(
      (marketplaceData?.servers ?? [])
        .filter((s) => s.recipe_hash)
        .map((s) => [installedKey(s.source_id, s.id), s.recipe_hash]),
    );
    const result = new Set<string>();
    for (const entry of installedData?.installed ?? []) {
      const key = installedKey(entry.source_id, entry.id);
      const current = currentHashes.get(key);
      if (entry.recipe_hash && current && entry.recipe_hash !== current) {
        result.add(key);
      }
    }
    return result;
  }, [marketplaceData, installedData]);

  const handleUpdate = async (configPath: string) => {
    setInstallError(null);
    try {
      await updateServer({ config_path: configPath }).unwrap();
      dispatch(integrationsApi.util.invalidateTags(["INTEGRATIONS"]));
    } catch (err) {
      setInstallError(installErrorMessage(err));
    }
  };

  const handleUninstall = async (configPath: string) => {
    setInstallError(null);
    try {
      await uninstallServer({ config_path: configPath }).unwrap();
      dispatch(integrationsApi.util.invalidateTags(["INTEGRATIONS"]));
    } catch (err) {
      setInstallError(installErrorMessage(err));
    }
  };

  const pagination = marketplaceData?.pagination;
  const totalPages = pagination
    ? Math.ceil(pagination.total / pagination.page_size)
    : 1;

  const handleInstall = async (server: MCPServer) => {
    if (requiredEnvKeys(server).length > 0) {
      // Never write a config with silently empty credentials: route to the
      // detail view where the required env vars can be filled in first.
      setInstallError(null);
      setSelectedServer(server);
      return;
    }
    setInstallingKey(installedKey(server.source_id, server.id));
    setInstallError(null);
    try {
      const result = await installServer({
        server_id: server.id,
        source_id: server.source_id,
      }).unwrap();
      dispatch(integrationsApi.util.invalidateTags(["INTEGRATIONS"]));
      dispatch(
        change({
          name: "integrations page",
          integrationPath: result.config_path,
        }),
      );
    } catch (err) {
      setInstallError(installErrorMessage(err));
    } finally {
      setInstallingKey(null);
    }
  };

  const handleSelectSource = (sourceId: string | null) => {
    setSelectedSource(sourceId);
    setInstallError(null);
    setPage(1);
  };

  const smitheryNeedsKey = sources.find(
    (s) => s.type === "smithery" && s.needs_api_key && !s.has_api_key,
  );

  const errorMessage =
    error && "data" in error
      ? String(error.data)
      : error
        ? "Failed to load marketplace"
        : null;

  if (selectedServer) {
    const detail = (
      <ServerDetail
        server={selectedServer}
        onBack={() => setSelectedServer(null)}
        onInstalled={handleConfigure}
      />
    );

    return embedded ? (
      detail
    ) : (
      <PageWrapper host={host}>
        <ScrollArea scrollbars="vertical" fullHeight>
          {detail}
        </ScrollArea>
      </PageWrapper>
    );
  }

  const content = (
    <div className={styles.pageStack}>
      {!embedded && (
        <div className={styles.header}>
          <Button
            size="sm"
            variant="ghost"
            leftIcon={ArrowLeft}
            onClick={backFromMarketplace}
          >
            Back
          </Button>
          <h2 className={styles.title}>MCP Marketplace</h2>
        </div>
      )}

      <div className={styles.toolbar}>
        <div className={styles.searchRow}>
          <div className={styles.searchWrap}>
            <FieldText
              aria-label="Search servers"
              className={styles.searchInput}
              placeholder="Search servers…"
              value={search}
              onChange={setSearch}
            />
          </div>
          <Icon icon={Search} tone="muted" />
        </div>

        {sources.length > 0 && (
          <SourceSelector
            sources={sources}
            selectedSource={selectedSource}
            onSelectSource={handleSelectSource}
            onOpenSettings={() => setSettingsOpen(true)}
          />
        )}
      </div>

      {allTags.length > 0 && (
        <div className={styles.filterRow}>
          <Badge
            tone={selectedTag === null ? "accent" : "muted"}
            className={styles.tagFilter}
            role="button"
            tabIndex={0}
            onClick={() => setSelectedTag(null)}
          >
            All
          </Badge>
          {allTags.map((tag) => (
            <Badge
              key={tag}
              tone={selectedTag === tag ? "accent" : "muted"}
              className={styles.tagFilter}
              role="button"
              tabIndex={0}
              onClick={() => setSelectedTag(selectedTag === tag ? null : tag)}
            >
              {tag}
            </Badge>
          ))}
        </div>
      )}

      {smitheryNeedsKey && (
        <div className={styles.notice}>
          <Icon icon={Info} tone="warning" />
          <p className={styles.smallText}>
            Smithery source requires an API key.
          </p>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setSettingsOpen(true)}
          >
            Configure
          </Button>
        </div>
      )}

      {installError && (
        <div className={`${styles.notice} ${styles.noticeDanger}`}>
          <Icon icon={Info} tone="danger" />
          <p className={styles.smallText}>{installError}</p>
        </div>
      )}

      {errorMessage && (
        <ErrorState
          title="Failed to load MCP marketplace"
          description={errorMessage}
        />
      )}

      {isLoading && <LoadingState label="Loading MCP marketplace" />}

      {!isLoading && !errorMessage && filteredServers.length === 0 && (
        <EmptyState
          title="No servers found"
          description="Try another search term, tag, or source."
        />
      )}

      {!isLoading && filteredServers.length > 0 && (
        <VirtualizedGrid
          items={filteredServers}
          getItemKey={(server) => `${server.source_id}:${server.id}`}
          rowHeight={SERVER_CARD_HEIGHT}
          aria-label="MCP servers"
          renderItem={(server) => (
            <ServerCard
              server={server}
              isInstalled={installedKeys.has(
                installedKey(server.source_id, server.id),
              )}
              installedConfigPath={installedConfigPaths.get(
                installedKey(server.source_id, server.id),
              )}
              updateAvailable={updateAvailableByKey.has(
                installedKey(server.source_id, server.id),
              )}
              onUpdate={(configPath) => void handleUpdate(configPath)}
              onUninstall={(configPath) => void handleUninstall(configPath)}
              isInstalling={
                installingKey === installedKey(server.source_id, server.id)
              }
              onInstall={(s) => void handleInstall(s)}
              onViewDetail={(s) => setSelectedServer(s)}
              onConfigure={handleConfigure}
              sourceLabel={sourceMap.get(server.source_id)}
            />
          )}
        />
      )}

      {totalPages > 1 && (
        <div className={styles.pagination}>
          <Button
            size="sm"
            variant="soft"
            disabled={page <= 1}
            onClick={() => setPage((p) => p - 1)}
          >
            Prev
          </Button>
          <p className={styles.smallText}>
            Page {page} of {totalPages}
          </p>
          <Button
            size="sm"
            variant="soft"
            disabled={page >= totalPages}
            onClick={() => setPage((p) => p + 1)}
          >
            Next
          </Button>
        </div>
      )}
    </div>
  );

  return embedded ? (
    <>
      {content}
      <SourceSettings
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
        sources={sources}
      />
    </>
  ) : (
    <PageWrapper host={host}>
      <ScrollArea scrollbars="vertical" fullHeight>
        {content}
      </ScrollArea>

      <SourceSettings
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
        sources={sources}
      />
    </PageWrapper>
  );
};

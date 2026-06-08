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
} from "../../components/ui";
import {
  useGetMarketplaceQuery,
  useGetInstalledServersQuery,
  useInstallServerMutation,
} from "../../services/refact/mcpMarketplace";
import type {
  MCPServer,
  MarketplaceSource,
} from "../../services/refact/mcpMarketplace";
import { ServerCard } from "./ServerCard";
import { ServerDetail } from "./ServerDetail";
import { SourceSelector } from "./SourceSelector";
import { SourceSettings } from "./SourceSettings";
import styles from "./MCPMarketplace.module.css";
import type { Config } from "../Config/configSlice";
import { useAppDispatch } from "../../hooks";
import { integrationsApi } from "../../services/refact/integrations";
import { change } from "../Pages/pagesSlice";

const PAGE_SIZE = 20;

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
  const [selectedTag, setSelectedTag] = useState<string | null>(null);
  const [selectedSource, setSelectedSource] = useState<string | null>(null);
  const [selectedServer, setSelectedServer] = useState<MCPServer | null>(null);
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [page, setPage] = useState(1);

  const {
    data: marketplaceData,
    isLoading,
    error,
  } = useGetMarketplaceQuery({
    source: selectedSource ?? undefined,
    page,
    page_size: PAGE_SIZE,
  });
  const { data: installedData } = useGetInstalledServersQuery(undefined);
  const [installServer] = useInstallServerMutation();

  const sources = useMemo<MarketplaceSource[]>(
    () => marketplaceData?.sources ?? [],
    [marketplaceData?.sources],
  );

  const sourceMap = useMemo(() => {
    const map = new Map<string, string>();
    sources.forEach((s) => map.set(s.id, s.label));
    return map;
  }, [sources]);

  const installedIds = useMemo(
    () => new Set((installedData?.installed ?? []).map((s) => s.id)),
    [installedData],
  );

  const installedConfigPaths = useMemo(
    () =>
      new Map(
        (installedData?.installed ?? []).map((s) => [s.id, s.config_path]),
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
    const tagSet = new Set<string>();
    (marketplaceData?.servers ?? []).forEach((s) =>
      s.tags.forEach((t) => tagSet.add(t)),
    );
    return Array.from(tagSet).sort();
  }, [marketplaceData]);

  const filteredServers = useMemo(() => {
    const servers = marketplaceData?.servers ?? [];
    const q = search.toLowerCase();
    return servers.filter((s) => {
      const matchesSearch =
        !q ||
        s.name.toLowerCase().includes(q) ||
        s.description.toLowerCase().includes(q) ||
        s.tags.some((t) => t.toLowerCase().includes(q));
      const matchesTag = !selectedTag || s.tags.includes(selectedTag);
      return matchesSearch && matchesTag;
    });
  }, [marketplaceData, search, selectedTag]);

  const pagination = marketplaceData?.pagination;
  const totalPages = pagination
    ? Math.ceil(pagination.total / pagination.page_size)
    : 1;

  const handleInstall = async (server: MCPServer) => {
    setInstallingId(server.id);
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
    } finally {
      setInstallingId(null);
    }
  };

  const handleSelectSource = (sourceId: string | null) => {
    setSelectedSource(sourceId);
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
          <Icon icon={Info} tone="accent" />
          <p className={styles.smallText}>Smithery source requires an API key.</p>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setSettingsOpen(true)}
          >
            Configure
          </Button>
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
        <div className={styles.serverGrid}>
          {filteredServers.map((server) => (
            <ServerCard
              key={`${server.source_id}:${server.id}`}
              server={server}
              isInstalled={installedIds.has(server.id)}
              installedConfigPath={installedConfigPaths.get(server.id)}
              isInstalling={installingId === server.id}
              onInstall={(s) => void handleInstall(s)}
              onViewDetail={(s) => setSelectedServer(s)}
              onConfigure={handleConfigure}
              sourceLabel={sourceMap.get(server.source_id)}
            />
          ))}
        </div>
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

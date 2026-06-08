import React, { useMemo, useState } from "react";
import classNames from "classnames";
import { ArrowLeft, Info, Search } from "lucide-react";
import { PageWrapper } from "../../components/PageWrapper";
import { ScrollArea } from "../../components/ScrollArea";
import {
  Button,
  EmptyState,
  ErrorState,
  FieldText,
  Icon,
  LoadingState,
} from "../../components/ui";
import { useAppDispatch } from "../../hooks";
import type { Config } from "../Config/configSlice";
import type {
  ExtensionMarketplaceItem,
  ExtensionMarketplaceSource,
} from "../../services/refact/extensionsMarketplace";
import { useSaveExtensionMarketplaceSourceMutation } from "../../services/refact/extensionsMarketplace";
import { change, type Page } from "../Pages/pagesSlice";
import { MarketplaceItemCard } from "./MarketplaceItemCard";
import { MarketplaceInstallDialog } from "./MarketplaceInstallDialog";
import { MarketplaceSourceSelector } from "./MarketplaceSourceSelector";
import { MarketplaceSourceSettings } from "./MarketplaceSourceSettings";
import styles from "./ExtensionsMarketplace.module.css";

type ExtensionsMarketplaceProps = {
  host: Config["host"];
  title: string;
  kind: "skill" | "command" | "subagent";
  tabbed: Config["tabbed"];
  back: () => void;
  items: ExtensionMarketplaceItem[];
  sources: ExtensionMarketplaceSource[];
  isLoading: boolean;
  error: unknown;
  isInstalling: boolean;
  onInstall: (
    item: ExtensionMarketplaceItem,
    scope: "local" | "global",
    params?: Record<string, string>,
    overwrite?: boolean,
  ) => Promise<void>;
  onInstalled?: (item: ExtensionMarketplaceItem) => Page;
  hasProjectRoot: boolean;
};

export const ExtensionsMarketplace: React.FC<ExtensionsMarketplaceProps> = ({
  host,
  title,
  kind,
  back,
  items,
  sources,
  isLoading,
  error,
  isInstalling,
  onInstall,
  onInstalled,
  hasProjectRoot,
}) => {
  const dispatch = useAppDispatch();
  const [search, setSearch] = useState("");
  const [selectedSource, setSelectedSource] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [installingItem, setInstallingItem] =
    useState<ExtensionMarketplaceItem | null>(null);
  const [installError, setInstallError] = useState<string | null>(null);
  const [isConflict, setIsConflict] = useState(false);
  const [quickAddUrl, setQuickAddUrl] = useState("");
  const [quickAddError, setQuickAddError] = useState<string | null>(null);
  const [saveSource, { isLoading: isAddingSource }] =
    useSaveExtensionMarketplaceSourceMutation();

  const filteredItems = useMemo(() => {
    const q = search.toLowerCase();
    return items.filter((item) => {
      const sourceOk =
        selectedSource === null || item.source_id === selectedSource;
      const searchOk =
        q.length === 0 ||
        item.name.toLowerCase().includes(q) ||
        item.description.toLowerCase().includes(q) ||
        item.tags.some((tag) => tag.toLowerCase().includes(q));
      return sourceOk && searchOk;
    });
  }, [items, search, selectedSource]);

  const errorMessage =
    error && typeof error === "object" && "data" in error
      ? String((error as { data: unknown }).data)
      : error
        ? `Failed to load ${kind}s marketplace`
        : null;

  const handleQuickAdd = async () => {
    if (!quickAddUrl.trim()) return;
    setQuickAddError(null);
    const result = await saveSource({
      url: quickAddUrl.trim(),
      enabled: true,
    });
    if ("error" in result) {
      const message =
        result.error &&
        typeof result.error === "object" &&
        "data" in result.error
          ? String(result.error.data)
          : "Failed to add source";
      setQuickAddError(message);
      return;
    }
    setQuickAddUrl("");
  };

  const handleInstall = async (
    scope: "local" | "global",
    params: Record<string, string>,
    overwrite: boolean,
  ) => {
    if (!installingItem) return;
    setInstallError(null);
    setIsConflict(false);
    try {
      await onInstall(installingItem, scope, params, overwrite);
      dispatch(
        change(
          onInstalled
            ? onInstalled(installingItem)
            : {
                name: "extensions",
                tab: kind === "skill" ? "skills" : "commands",
                itemId: installingItem.name,
              },
        ),
      );
    } catch (err) {
      const status =
        err && typeof err === "object" && "status" in err
          ? (err as { status: number }).status
          : 0;
      if (status === 409) {
        setIsConflict(true);
      }
      if (err && typeof err === "object" && "data" in err) {
        setInstallError(String((err as { data: unknown }).data));
        return;
      }
      setInstallError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <PageWrapper host={host}>
      <ScrollArea scrollbars="vertical" fullHeight>
        <div className={styles.pageStack}>
          <div className={styles.header}>
            <Button
              variant="ghost"
              size="sm"
              leftIcon={ArrowLeft}
              onClick={back}
            >
              Back
            </Button>
            <h2 className={styles.title}>{title}</h2>
          </div>

          <div className={styles.quickAddRow}>
            <FieldText
              aria-label={`Search ${kind}s`}
              value={search}
              placeholder={`Search ${kind}s…`}
              onChange={setSearch}
              className={styles.grow}
            />
            <Icon icon={Search} tone="muted" />
          </div>

          <MarketplaceSourceSelector
            sources={sources}
            selectedSource={selectedSource}
            onSelectSource={setSelectedSource}
            onOpenSettings={() => setSettingsOpen(true)}
          />

          <div className={styles.quickAddSection}>
            <p className={styles.text}>Add GitHub Source by URL</p>
            <div className={styles.quickAddRow}>
              <FieldText
                aria-label="GitHub source URL"
                placeholder="https://github.com/owner/repo"
                value={quickAddUrl}
                onChange={setQuickAddUrl}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    void handleQuickAdd();
                  }
                }}
                className={styles.grow}
              />
              <Button
                variant="primary"
                onClick={() => void handleQuickAdd()}
                disabled={!quickAddUrl.trim() || isAddingSource}
                loading={isAddingSource}
              >
                {isAddingSource ? "Adding…" : "Add"}
              </Button>
            </div>
            {quickAddError && (
              <div className={classNames(styles.notice, styles.noticeDanger)}>
                <Icon icon={Info} tone="danger" />
                <p className={styles.smallText}>{quickAddError}</p>
              </div>
            )}
          </div>

          {errorMessage && (
            <ErrorState
              title={`Failed to load ${kind}s marketplace`}
              description={errorMessage}
            />
          )}

          {isLoading && <LoadingState label={`Loading ${kind}s marketplace`} />}

          {!isLoading && !errorMessage && filteredItems.length === 0 && (
            <EmptyState
              title={`No ${kind}s found`}
              description="Try another search term or source."
            />
          )}

          {!isLoading && filteredItems.length > 0 && (
            <div className={styles.grid}>
              {filteredItems.map((item) => (
                <MarketplaceItemCard
                  key={`${item.source_id}:${item.id}`}
                  item={item}
                  isInstalling={
                    isInstalling &&
                    installingItem?.id === item.id &&
                    installingItem.source_id === item.source_id
                  }
                  onInstall={(next) => {
                    setInstallError(null);
                    setInstallingItem(next);
                  }}
                />
              ))}
            </div>
          )}
        </div>
      </ScrollArea>

      <MarketplaceSourceSettings
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
        sources={sources}
      />
      <MarketplaceInstallDialog
        open={installingItem !== null}
        item={installingItem}
        hasProjectRoot={hasProjectRoot}
        isInstalling={isInstalling}
        isConflict={isConflict}
        error={installError}
        onOpenChange={(open) => {
          if (!open) {
            setInstallingItem(null);
            setInstallError(null);
            setIsConflict(false);
          }
        }}
        onInstall={(scope, params, overwrite) =>
          void handleInstall(scope, params, overwrite)
        }
      />
    </PageWrapper>
  );
};

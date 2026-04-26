import React, { useMemo, useState } from "react";
import {
  Button,
  Callout,
  Flex,
  Heading,
  Text,
  TextField,
} from "@radix-ui/themes";
import {
  ArrowLeftIcon,
  InfoCircledIcon,
  MagnifyingGlassIcon,
} from "@radix-ui/react-icons";
import { PageWrapper } from "../../components/PageWrapper";
import { ScrollArea } from "../../components/ScrollArea";
import { Spinner } from "../../components/Spinner";
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
    <PageWrapper host={host} style={{ padding: "var(--space-4)" }}>
      <ScrollArea scrollbars="vertical" fullHeight>
        <Flex direction="column" gap="4">
          <Flex align="center" gap="3">
            <Button variant="ghost" size="1" onClick={back}>
              <ArrowLeftIcon />
              Back
            </Button>
            <Heading size="4">{title}</Heading>
          </Flex>

          <TextField.Root
            size="2"
            placeholder={`Search ${kind}s…`}
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          >
            <TextField.Slot>
              <MagnifyingGlassIcon />
            </TextField.Slot>
          </TextField.Root>

          <MarketplaceSourceSelector
            sources={sources}
            selectedSource={selectedSource}
            onSelectSource={setSelectedSource}
            onOpenSettings={() => setSettingsOpen(true)}
          />

          <Flex direction="column" gap="2">
            <Text size="2" weight="bold">
              Add GitHub Source by URL
            </Text>
            <Flex gap="2">
              <TextField.Root
                size="2"
                placeholder="https://github.com/owner/repo"
                value={quickAddUrl}
                onChange={(event) => setQuickAddUrl(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    void handleQuickAdd();
                  }
                }}
              />
              <Button
                size="2"
                onClick={() => void handleQuickAdd()}
                disabled={!quickAddUrl.trim() || isAddingSource}
              >
                {isAddingSource ? "Adding…" : "Add"}
              </Button>
            </Flex>
            {quickAddError && (
              <Callout.Root color="red" size="1">
                <Callout.Icon>
                  <InfoCircledIcon />
                </Callout.Icon>
                <Callout.Text>{quickAddError}</Callout.Text>
              </Callout.Root>
            )}
          </Flex>

          {errorMessage && (
            <Callout.Root color="red" size="1">
              <Callout.Icon>
                <InfoCircledIcon />
              </Callout.Icon>
              <Callout.Text>{errorMessage}</Callout.Text>
            </Callout.Root>
          )}

          {isLoading && <Spinner spinning />}

          {!isLoading && !errorMessage && filteredItems.length === 0 && (
            <Text size="2" color="gray" align="center">
              No {kind}s found
            </Text>
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
        </Flex>
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

import React, { useState } from "react";
import {
  Button,
  Callout,
  Dialog,
  Flex,
  Switch,
  Text,
  TextField,
} from "@radix-ui/themes";
import { InfoCircledIcon, ReloadIcon, TrashIcon } from "@radix-ui/react-icons";
import type { ExtensionMarketplaceSource } from "../../services/refact/extensionsMarketplace";
import {
  useConfigureExtensionMarketplaceSourceMutation,
  useDeleteExtensionMarketplaceSourceMutation,
  useRefreshExtensionMarketplaceSourceMutation,
  useSaveExtensionMarketplaceSourceMutation,
} from "../../services/refact/extensionsMarketplace";
import styles from "./ExtensionsMarketplace.module.css";

type MarketplaceSourceSettingsProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sources: ExtensionMarketplaceSource[];
};

const AddSourceForm: React.FC = () => {
  const [url, setUrl] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saveSource] = useSaveExtensionMarketplaceSourceMutation();

  const handleAdd = async () => {
    if (!url.trim()) return;
    const result = await saveSource({ url: url.trim(), enabled: true });
    if ("error" in result) {
      const message =
        result.error &&
        typeof result.error === "object" &&
        "data" in result.error
          ? String(result.error.data)
          : "Failed to add source";
      setError(message);
      return;
    }
    setUrl("");
    setError(null);
  };

  return (
    <Flex direction="column" gap="2" className={styles.addSourceSection}>
      <Text size="2" weight="bold">
        Quick-add GitHub Source
      </Text>
      {error && (
        <Callout.Root color="red" size="1">
          <Callout.Icon>
            <InfoCircledIcon />
          </Callout.Icon>
          <Callout.Text>{error}</Callout.Text>
        </Callout.Root>
      )}
      <TextField.Root
        size="1"
        placeholder="https://github.com/owner/repo"
        value={url}
        onChange={(event) => setUrl(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            void handleAdd();
          }
        }}
      />
      <Button size="1" onClick={() => void handleAdd()} disabled={!url.trim()}>
        Add by URL
      </Button>
    </Flex>
  );
};

export const MarketplaceSourceSettings: React.FC<
  MarketplaceSourceSettingsProps
> = ({ open, onOpenChange, sources }) => {
  const [deleteSource] = useDeleteExtensionMarketplaceSourceMutation();
  const [configureSource] = useConfigureExtensionMarketplaceSourceMutation();
  const [refreshSource, { isLoading: isRefreshing }] =
    useRefreshExtensionMarketplaceSourceMutation();

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Content style={{ maxWidth: 520 }}>
        <Dialog.Title>Marketplace Sources</Dialog.Title>
        <Flex direction="column" gap="1">
          {sources.map((source) => (
            <div className={styles.sourceRow} key={source.id}>
              <Switch
                size="1"
                checked={source.enabled}
                onCheckedChange={(enabled) =>
                  void configureSource({ id: source.id, enabled })
                }
              />
              <Flex direction="column" gap="0" className={styles.sourceLabel}>
                <Text size="2">{source.label}</Text>
                <Text size="1" color="gray">
                  {source.description.length > 0
                    ? source.description
                    : source.repo_url ?? "Marketplace source"}
                </Text>
                {!source.removable && (
                  <Text size="1" color="gray">
                    Built-in
                  </Text>
                )}
                {source.error && (
                  <Text size="1" color="red">
                    {source.error}
                  </Text>
                )}
              </Flex>
              {source.source_kind !== "builtin_embedded" && source.enabled && (
                <Button
                  size="1"
                  variant="ghost"
                  color="gray"
                  disabled={isRefreshing}
                  onClick={() => void refreshSource({ id: source.id })}
                  title="Re-sync from source"
                >
                  <ReloadIcon />
                </Button>
              )}
              {source.removable && (
                <Button
                  size="1"
                  variant="ghost"
                  color="red"
                  onClick={() => void deleteSource({ id: source.id })}
                >
                  <TrashIcon />
                </Button>
              )}
            </div>
          ))}
        </Flex>
        <hr className={styles.divider} />
        <AddSourceForm />
        <Flex justify="end" mt="4">
          <Dialog.Close>
            <Button variant="soft">Close</Button>
          </Dialog.Close>
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};

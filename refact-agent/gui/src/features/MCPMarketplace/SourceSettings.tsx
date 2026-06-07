import React, { useState } from "react";
import {
  Button,
  Dialog,
  Flex,
  Switch,
  Text,
  TextField,
  Callout,
} from "@radix-ui/themes";
import { TrashIcon, InfoCircledIcon } from "@radix-ui/react-icons";
import type { MarketplaceSource } from "../../services/refact/mcpMarketplace";
import {
  useDeleteMarketplaceSourceMutation,
  useConfigureMarketplaceSourceMutation,
  useSaveMarketplaceSourceMutation,
} from "../../services/refact/mcpMarketplace";
import styles from "./SourceSettings.module.css";

type SourceSettingsProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sources: MarketplaceSource[];
};

type SmitheryKeyFormProps = {
  source: MarketplaceSource;
};

const SmitheryKeyForm: React.FC<SmitheryKeyFormProps> = ({ source }) => {
  const [apiKey, setApiKey] = useState("");
  const [configureSource] = useConfigureMarketplaceSourceMutation();

  const handleSave = async () => {
    if (!apiKey.trim()) return;
    await configureSource({
      id: source.id,
      api_key: apiKey.trim(),
      enabled: true,
    });
    setApiKey("");
  };

  if (source.has_api_key) {
    return (
      <Flex direction="column" gap="1" className={styles.apiKeySection}>
        <Text size="1" color="gray">
          API Key: configured
        </Text>
        <Button
          size="1"
          variant="ghost"
          color="red"
          onClick={() =>
            void configureSource({ id: source.id, api_key: "", enabled: false })
          }
        >
          Remove API Key
        </Button>
      </Flex>
    );
  }

  return (
    <Flex direction="column" gap="1" className={styles.apiKeySection}>
      <Text size="1" color="gray">
        API Key required — get one at smithery.ai/account/api-keys
      </Text>
      <Flex gap="2">
        <TextField.Root
          size="1"
          type="password"
          placeholder="Enter API Key…"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          className={styles.grow}
        />
        <Button
          size="1"
          onClick={() => void handleSave()}
          disabled={!apiKey.trim()}
        >
          Save
        </Button>
      </Flex>
    </Flex>
  );
};

type AddCustomSourceFormProps = {
  onAdded: () => void;
};

const AddCustomSourceForm: React.FC<AddCustomSourceFormProps> = ({
  onAdded,
}) => {
  const [label, setLabel] = useState("");
  const [url, setUrl] = useState("");
  const [saveSource] = useSaveMarketplaceSourceMutation();
  const [error, setError] = useState<string | null>(null);

  const handleAdd = async () => {
    if (!label.trim() || !url.trim()) return;
    const id = label
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-");
    const result = await saveSource({
      id,
      label: label.trim(),
      type: "refact_index",
      url: url.trim(),
      enabled: true,
    });
    if ("error" in result) {
      setError("Failed to add source");
    } else {
      setLabel("");
      setUrl("");
      setError(null);
      onAdded();
    }
  };

  return (
    <Flex direction="column" gap="2" className={styles.addSourceSection}>
      <Text size="2" weight="bold">
        Add Custom Source
      </Text>
      {error && (
        <Callout.Root color="red" size="1">
          <Callout.Icon>
            <InfoCircledIcon />
          </Callout.Icon>
          <Callout.Text>{error}</Callout.Text>
        </Callout.Root>
      )}
      <Flex direction="column" gap="1">
        <Text size="1" color="gray">
          Label
        </Text>
        <TextField.Root
          size="1"
          placeholder="My Registry"
          value={label}
          onChange={(e) => setLabel(e.target.value)}
        />
      </Flex>
      <Flex direction="column" gap="1">
        <Text size="1" color="gray">
          URL
        </Text>
        <TextField.Root
          size="1"
          placeholder="https://example.com/mcp-index.json"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
        />
      </Flex>
      <Button
        size="1"
        onClick={() => void handleAdd()}
        disabled={!label.trim() || !url.trim()}
      >
        Add Source
      </Button>
    </Flex>
  );
};

export const SourceSettings: React.FC<SourceSettingsProps> = ({
  open,
  onOpenChange,
  sources,
}) => {
  const [deleteSource] = useDeleteMarketplaceSourceMutation();
  const [configureSource] = useConfigureMarketplaceSourceMutation();

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Content className={styles.dialogContent}>
        <Dialog.Title>Marketplace Sources</Dialog.Title>
        <Flex direction="column" gap="1">
          {sources.map((source) => (
            <Flex key={source.id} direction="column">
              <div className={styles.sourceRow}>
                <Switch
                  size="1"
                  checked={source.enabled}
                  disabled={!source.removable}
                  onCheckedChange={(checked) =>
                    void configureSource({ id: source.id, enabled: checked })
                  }
                />
                <Flex direction="column" gap="0" className={styles.sourceLabel}>
                  <Text size="2">{source.label}</Text>
                  {source.status === "error" && source.error && (
                    <Text size="1" color="red">
                      {source.error}
                    </Text>
                  )}
                  {!source.removable && (
                    <Text size="1" color="gray">
                      Built-in
                    </Text>
                  )}
                </Flex>
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
              {source.type === "smithery" && source.enabled && (
                <SmitheryKeyForm source={source} />
              )}
            </Flex>
          ))}
        </Flex>
        <hr className={styles.divider} />
        <AddCustomSourceForm onAdded={() => undefined} />
        <Flex justify="end" mt="4">
          <Dialog.Close>
            <Button variant="soft">Close</Button>
          </Dialog.Close>
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};

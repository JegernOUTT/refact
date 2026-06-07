import React, { useState } from "react";
import classNames from "classnames";
import { Info, Trash } from "lucide-react";
import type { MarketplaceSource } from "../../services/refact/mcpMarketplace";
import {
  useDeleteMarketplaceSourceMutation,
  useConfigureMarketplaceSourceMutation,
  useSaveMarketplaceSourceMutation,
} from "../../services/refact/mcpMarketplace";
import { Button, Dialog, FieldText, Icon, Switch } from "../../components/ui";
import styles from "./SourceSettings.module.css";
import sharedStyles from "./MCPMarketplace.module.css";

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
      <div className={styles.apiKeySection}>
        <p className={sharedStyles.smallText}>API Key: configured</p>
        <Button
          size="sm"
          variant="danger"
          onClick={() =>
            void configureSource({ id: source.id, api_key: "", enabled: false })
          }
        >
          Remove API Key
        </Button>
      </div>
    );
  }

  return (
    <div className={styles.apiKeySection}>
      <p className={sharedStyles.smallText}>
        API Key required — get one at smithery.ai/account/api-keys
      </p>
      <div className={sharedStyles.searchRow}>
        <FieldText
          type="password"
          placeholder="Enter API Key…"
          value={apiKey}
          onChange={setApiKey}
          className={styles.grow}
        />
        <Button
          size="sm"
          variant="primary"
          onClick={() => void handleSave()}
          disabled={!apiKey.trim()}
        >
          Save
        </Button>
      </div>
    </div>
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
    <div className={styles.addSourceSection}>
      <p className={sharedStyles.text}>Add Custom Source</p>
      {error && (
        <div className={classNames(sharedStyles.notice, sharedStyles.noticeDanger)}>
          <Icon icon={Info} tone="danger" />
          <p className={sharedStyles.smallText}>{error}</p>
        </div>
      )}
      <label className={sharedStyles.configField}>
        <span className={sharedStyles.smallText}>Label</span>
        <FieldText placeholder="My Registry" value={label} onChange={setLabel} />
      </label>
      <label className={sharedStyles.configField}>
        <span className={sharedStyles.smallText}>URL</span>
        <FieldText
          placeholder="https://example.com/mcp-index.json"
          value={url}
          onChange={setUrl}
        />
      </label>
      <Button
        size="sm"
        variant="primary"
        onClick={() => void handleAdd()}
        disabled={!label.trim() || !url.trim()}
      >
        Add Source
      </Button>
    </div>
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
    <Dialog open={open} onOpenChange={onOpenChange}>
      <Dialog.Content className={styles.dialogContent}>
        <Dialog.Title>Marketplace Sources</Dialog.Title>
        <div className={sharedStyles.configStack}>
          {sources.map((source) => (
            <div key={source.id}>
              <div className={styles.sourceRow}>
                <Switch
                  checked={source.enabled}
                  disabled={!source.removable}
                  onCheckedChange={(checked) =>
                    void configureSource({ id: source.id, enabled: checked })
                  }
                />
                <div className={styles.sourceLabel}>
                  <p className={sharedStyles.text}>{source.label}</p>
                  {source.status === "error" && source.error && (
                    <p className={sharedStyles.dangerText}>{source.error}</p>
                  )}
                  {!source.removable && (
                    <p className={sharedStyles.smallText}>Built-in</p>
                  )}
                </div>
                {source.removable && (
                  <Button
                    size="sm"
                    variant="danger"
                    leftIcon={Trash}
                    onClick={() => void deleteSource({ id: source.id })}
                  >
                    Remove
                  </Button>
                )}
              </div>
              {source.type === "smithery" && source.enabled && (
                <SmitheryKeyForm source={source} />
              )}
            </div>
          ))}
        </div>
        <hr className={styles.divider} />
        <AddCustomSourceForm onAdded={() => undefined} />
        <div className={sharedStyles.cardActionRow}>
          <Dialog.Close asChild>
            <Button variant="soft">Close</Button>
          </Dialog.Close>
        </div>
      </Dialog.Content>
    </Dialog>
  );
};

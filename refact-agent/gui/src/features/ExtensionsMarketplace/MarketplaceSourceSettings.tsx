import React, { useState } from "react";
import classNames from "classnames";
import { Info, RefreshCw, Trash } from "lucide-react";
import type { ExtensionMarketplaceSource } from "../../services/refact/extensionsMarketplace";
import {
  useConfigureExtensionMarketplaceSourceMutation,
  useDeleteExtensionMarketplaceSourceMutation,
  useRefreshExtensionMarketplaceSourceMutation,
  useSaveExtensionMarketplaceSourceMutation,
} from "../../services/refact/extensionsMarketplace";
import { Button, Dialog, FieldText, Icon, Switch } from "../../components/ui";
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
    <div className={styles.addSourceSection}>
      <p className={styles.text}>Quick-add GitHub Source</p>
      {error && (
        <div className={classNames(styles.notice, styles.noticeDanger)}>
          <Icon icon={Info} tone="danger" />
          <p className={styles.smallText}>{error}</p>
        </div>
      )}
      <FieldText
        placeholder="https://github.com/owner/repo"
        value={url}
        onChange={setUrl}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            void handleAdd();
          }
        }}
      />
      <Button variant="primary" size="sm" onClick={() => void handleAdd()} disabled={!url.trim()}>
        Add by URL
      </Button>
    </div>
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
    <Dialog open={open} onOpenChange={onOpenChange}>
      <Dialog.Content className={styles.sourceSettingsDialogContent}>
        <Dialog.Title>Marketplace Sources</Dialog.Title>
        <div className={styles.dialogStack}>
          <div>
            {sources.map((source) => (
              <div className={styles.sourceRow} key={source.id}>
                <Switch
                  checked={source.enabled}
                  onCheckedChange={(enabled) =>
                    void configureSource({ id: source.id, enabled })
                  }
                />
                <div className={styles.sourceLabel}>
                  <p className={styles.text}>{source.label}</p>
                  <p className={styles.smallText}>
                    {source.description.length > 0
                      ? source.description
                      : source.repo_url ?? "Marketplace source"}
                  </p>
                  {!source.removable && <p className={styles.smallText}>Built-in</p>}
                  {source.error && <p className={styles.errorText}>{source.error}</p>}
                </div>
                {source.source_kind !== "builtin_embedded" && source.enabled && (
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={isRefreshing}
                    loading={isRefreshing}
                    leftIcon={RefreshCw}
                    onClick={() => void refreshSource({ id: source.id })}
                    title="Re-sync from source"
                  >
                    Sync
                  </Button>
                )}
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
            ))}
          </div>
          <hr className={styles.divider} />
          <AddSourceForm />
          <div className={styles.cardActionRow}>
            <Dialog.Close asChild>
              <Button variant="soft">Close</Button>
            </Dialog.Close>
          </div>
        </div>
      </Dialog.Content>
    </Dialog>
  );
};

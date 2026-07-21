import React, { useState } from "react";
import classNames from "classnames";
import { ArrowLeft, Check, ExternalLink, Info } from "lucide-react";
import type { MCPServer } from "../../services/refact/mcpMarketplace";
import {
  useInstallServerMutation,
  useGetInstalledServersQuery,
} from "../../services/refact/mcpMarketplace";
import { Badge, Button, FieldText, Icon } from "../../components/ui";
import { requiredEnvKeys } from "./requiredEnv";
import { installErrorMessage, installedKey } from "./installError";
import styles from "./MCPMarketplace.module.css";

type ServerDetailProps = {
  server: MCPServer;
  onBack: () => void;
  onInstalled?: (configPath: string) => void;
};

export const ServerDetail: React.FC<ServerDetailProps> = ({
  server,
  onBack,
  onInstalled,
}) => {
  const defaultEnv = server.install_recipe.env ?? {};
  const [envValues, setEnvValues] = useState<
    Record<string, string | undefined>
  >(Object.fromEntries(Object.entries(defaultEnv).map(([k, v]) => [k, v])));

  const [installServer, { isLoading, isSuccess, error }] =
    useInstallServerMutation();
  const { data: installedData } = useGetInstalledServersQuery(undefined);

  const isInstalled =
    installedData?.installed.some(
      (s) =>
        installedKey(s.source_id, s.id) ===
        installedKey(server.source_id, server.id),
    ) ?? false;

  const requiredKeys = requiredEnvKeys(server);
  const missingRequiredKeys = requiredKeys.filter(
    (key) => !(envValues[key] ?? "").trim(),
  );

  const handleInstall = async () => {
    const definedEnv = Object.fromEntries(
      Object.entries(envValues).filter(
        (e): e is [string, string] => e[1] !== undefined,
      ),
    );
    const configOverrides =
      Object.keys(definedEnv).length > 0 ? { env: definedEnv } : undefined;
    try {
      const result = await installServer({
        server_id: server.id,
        source_id: server.source_id,
        config_overrides: configOverrides,
      }).unwrap();
      onInstalled?.(result.config_path);
    } catch {
      // The mutation error state renders the failure notice below.
    }
  };

  const errorMessage = error ? installErrorMessage(error) : null;

  return (
    <div className={styles.detailRoot}>
      <div className={styles.header}>
        <Button variant="ghost" size="sm" leftIcon={ArrowLeft} onClick={onBack}>
          Back
        </Button>
      </div>

      <div className={styles.detailHeader}>
        <div className={styles.serverIconPlaceholderLarge}>
          {server.name.charAt(0).toUpperCase()}
        </div>
        <div className={styles.detailTitle}>
          <h2 className={styles.title}>{server.name}</h2>
          <p className={styles.mutedText}>by {server.publisher}</p>
          <div className={styles.detailMeta}>
            <Badge tone="accent" className={styles.neutralBadge}>
              {server.transport}
            </Badge>
            {server.homepage && (
              <Button
                size="sm"
                variant="ghost"
                rightIcon={ExternalLink}
                onClick={() =>
                  window.open(server.homepage, "_blank", "noopener,noreferrer")
                }
              >
                Homepage
              </Button>
            )}
          </div>
        </div>
      </div>

      <p className={styles.detailDescription}>{server.description}</p>

      {server.tags.length > 0 && (
        <div className={styles.detailTags}>
          {server.tags.map((tag) => (
            <Badge key={tag} tone="muted">
              {tag}
            </Badge>
          ))}
        </div>
      )}

      {Object.keys(defaultEnv).length > 0 && (
        <div className={styles.configStack}>
          <p className={styles.text}>Configuration</p>
          {Object.keys(defaultEnv).map((key) => (
            <label key={key} className={styles.configField}>
              <span className={styles.smallText}>
                {key}
                {requiredKeys.includes(key) ? " (required)" : ""}
              </span>
              <FieldText
                value={envValues[key] ?? ""}
                onChange={(nextValue) =>
                  setEnvValues((prev) => ({ ...prev, [key]: nextValue }))
                }
                placeholder={defaultEnv[key]}
              />
            </label>
          ))}
          {missingRequiredKeys.length > 0 && (
            <p className={styles.mutedText}>
              Fill in {missingRequiredKeys.join(", ")} to install this server.
            </p>
          )}
        </div>
      )}

      {errorMessage && (
        <div className={classNames(styles.notice, styles.noticeDanger)}>
          <Icon icon={Info} tone="danger" />
          <p className={styles.smallText}>{errorMessage}</p>
        </div>
      )}

      {isSuccess && (
        <div className={classNames(styles.notice, styles.noticeSuccess)}>
          <Icon icon={Check} tone="success" />
          <p className={styles.smallText}>Server installed successfully!</p>
        </div>
      )}

      {isInstalled && !isSuccess && (
        <div className={classNames(styles.notice, styles.noticeSuccess)}>
          <Icon icon={Check} tone="success" />
          <p className={styles.smallText}>Already installed</p>
        </div>
      )}

      {!isInstalled && (
        <Button
          variant="primary"
          onClick={() => void handleInstall()}
          disabled={isLoading || missingRequiredKeys.length > 0}
          loading={isLoading}
          className={styles.alignStart}
        >
          {isLoading ? "Installing…" : "Install"}
        </Button>
      )}
    </div>
  );
};

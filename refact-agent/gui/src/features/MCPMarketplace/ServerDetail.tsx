import React, { useState } from "react";
import classNames from "classnames";
import { ArrowLeft, Check, ExternalLink, Info } from "lucide-react";
import type { MCPServer } from "../../services/refact/mcpMarketplace";
import {
  useInstallServerMutation,
  useGetInstalledServersQuery,
} from "../../services/refact/mcpMarketplace";
import { Badge, Button, FieldText, Icon } from "../../components/ui";
import styles from "./MCPMarketplace.module.css";

type ServerDetailProps = {
  server: MCPServer;
  onBack: () => void;
};

export const ServerDetail: React.FC<ServerDetailProps> = ({
  server,
  onBack,
}) => {
  const defaultEnv = server.install_recipe.env ?? {};
  const [envValues, setEnvValues] = useState<
    Record<string, string | undefined>
  >(Object.fromEntries(Object.entries(defaultEnv).map(([k, v]) => [k, v])));

  const [installServer, { isLoading, isSuccess, error }] =
    useInstallServerMutation();
  const { data: installedData } = useGetInstalledServersQuery(undefined);

  const isInstalled =
    installedData?.installed.some((s) => s.id === server.id) ?? false;

  const handleInstall = async () => {
    const definedEnv = Object.fromEntries(
      Object.entries(envValues).filter(
        (e): e is [string, string] => e[1] !== undefined,
      ),
    );
    const configOverrides =
      Object.keys(definedEnv).length > 0 ? { env: definedEnv } : undefined;
    await installServer({
      server_id: server.id,
      source_id: server.source_id,
      config_overrides: configOverrides,
    });
  };

  const errorMessage =
    error && "data" in error
      ? String(error.data)
      : error
        ? "Installation failed"
        : null;

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
            <Badge tone="accent">{server.transport}</Badge>
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
              <span className={styles.smallText}>{key}</span>
              <FieldText
                value={envValues[key] ?? ""}
                onChange={(nextValue) =>
                  setEnvValues((prev) => ({ ...prev, [key]: nextValue }))
                }
                placeholder={defaultEnv[key]}
              />
            </label>
          ))}
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
          disabled={isLoading}
          loading={isLoading}
          className={styles.alignStart}
        >
          {isLoading ? "Installing…" : "Install"}
        </Button>
      )}
    </div>
  );
};

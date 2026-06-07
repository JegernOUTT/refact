import React, { useEffect, useState } from "react";
import classNames from "classnames";
import { AlertTriangle, File, Globe } from "lucide-react";
import type { ExtensionMarketplaceItem } from "../../services/refact/extensionsMarketplace";
import {
  Badge,
  Button,
  Dialog,
  FieldText,
  Icon,
  SegmentedControl,
} from "../../components/ui";
import styles from "./ExtensionsMarketplace.module.css";

type MarketplaceInstallDialogProps = {
  open: boolean;
  item: ExtensionMarketplaceItem | null;
  hasProjectRoot: boolean;
  isInstalling: boolean;
  isConflict: boolean;
  error: string | null;
  onOpenChange: (open: boolean) => void;
  onInstall: (
    scope: "local" | "global",
    params: Record<string, string>,
    overwrite: boolean,
  ) => void;
};

export const MarketplaceInstallDialog: React.FC<
  MarketplaceInstallDialogProps
> = ({
  open,
  item,
  hasProjectRoot,
  isInstalling,
  isConflict,
  error,
  onOpenChange,
  onInstall,
}) => {
  const [scope, setScope] = useState<"local" | "global">(
    hasProjectRoot ? "local" : "global",
  );
  const [paramValues, setParamValues] = useState<
    Partial<Record<string, string>>
  >({});

  useEffect(() => {
    setScope(hasProjectRoot ? "local" : "global");
  }, [hasProjectRoot, item?.id]);

  useEffect(() => {
    if (item?.params && item.params.length > 0) {
      const defaults: Record<string, string> = {};
      for (const p of item.params) {
        defaults[p.name] = p.default ?? "";
      }
      setParamValues(defaults);
    } else {
      setParamValues({});
    }
  }, [item?.id, item?.params]);

  const handleInstallClick = (overwrite: boolean) => {
    const params: Record<string, string> = {};
    for (const [k, v] of Object.entries(paramValues)) {
      if (v !== undefined) params[k] = v;
    }
    onInstall(scope, params, overwrite);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <Dialog.Content className={styles.installDialogContent}>
        <Dialog.Title>Install {item?.kind}</Dialog.Title>
        <div className={styles.dialogStack}>
          <div className={styles.dialogStack}>
            <p className={styles.text}>{item?.name}</p>
            <p className={styles.mutedText}>
              {item?.description && item.description.length > 0
                ? item.description
                : "No description"}
            </p>
            {item?.kind === "subagent" && (
              <p className={styles.smallText}>
                Installs as editable Refact YAML under `.refact/subagents` or
                your global config.
              </p>
            )}
          </div>

          <div className={styles.filterRow}>
            <Badge tone="accent">{item?.source_label}</Badge>
            {item?.tags.map((tag) => (
              <Badge key={tag} tone="muted">
                {tag}
              </Badge>
            ))}
          </div>

          <div className={styles.installScope}>
            <p className={styles.smallText}>Install to:</p>
            {hasProjectRoot ? (
              <SegmentedControl
                size="sm"
                value={scope}
                onValueChange={(value) => setScope(value as "local" | "global")}
                options={[
                  {
                    value: "global",
                    label: (
                      <span className={styles.dialogHeader}>
                        <Icon icon={Globe} size="sm" tone="muted" /> Global
                      </span>
                    ),
                  },
                  {
                    value: "local",
                    label: (
                      <span className={styles.dialogHeader}>
                        <Icon icon={File} size="sm" tone="muted" /> Project
                      </span>
                    ),
                  },
                ]}
              />
            ) : (
              <Badge tone="accent">
                <span className={styles.dialogHeader}>
                  <Icon icon={Globe} size="sm" tone="accent" /> Global only (no project open)
                </span>
              </Badge>
            )}
          </div>

          {item?.params && item.params.length > 0 && (
            <div className={styles.paramsStack}>
              <p className={styles.smallText}>Parameters</p>
              {item.params.map((param) => (
                <label key={param.name} className={styles.paramField}>
                  <span className={styles.smallText}>
                    {param.label || param.name}
                    {param.required && " *"}
                  </span>
                  <FieldText
                    placeholder={param.default ?? `Enter ${param.name}…`}
                    value={paramValues[param.name] ?? ""}
                    onChange={(nextValue) =>
                      setParamValues((prev) => ({
                        ...prev,
                        [param.name]: nextValue,
                      }))
                    }
                  />
                </label>
              ))}
            </div>
          )}

          {isConflict && (
            <div className={classNames(styles.notice, styles.noticeWarning)}>
              <Icon icon={AlertTriangle} tone="warning" />
              <p className={styles.smallText}>
                Already installed in this scope. Click <strong>Overwrite</strong> to replace it,
                or switch the scope above.
              </p>
            </div>
          )}

          {error && !isConflict && <p className={styles.errorText}>{error}</p>}
        </div>

        <div className={styles.cardActionRow}>
          <Dialog.Close asChild>
            <Button variant="soft">Cancel</Button>
          </Dialog.Close>
          {isConflict ? (
            <Button
              variant="primary"
              onClick={() => handleInstallClick(true)}
              disabled={!item || isInstalling}
              loading={isInstalling}
            >
              {isInstalling ? "Overwriting…" : "Overwrite"}
            </Button>
          ) : (
            <Button
              variant="primary"
              onClick={() => handleInstallClick(false)}
              disabled={!item || isInstalling}
              loading={isInstalling}
            >
              {isInstalling ? "Installing…" : "Install"}
            </Button>
          )}
        </div>
      </Dialog.Content>
    </Dialog>
  );
};

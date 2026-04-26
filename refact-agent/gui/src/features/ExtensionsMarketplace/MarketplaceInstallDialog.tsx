import React, { useEffect, useState } from "react";
import {
  Badge,
  Button,
  Callout,
  Dialog,
  Flex,
  SegmentedControl,
  Text,
  TextField,
} from "@radix-ui/themes";
import {
  ExclamationTriangleIcon,
  FileIcon,
  GlobeIcon,
} from "@radix-ui/react-icons";
import type { ExtensionMarketplaceItem } from "../../services/refact/extensionsMarketplace";
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
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Content style={{ maxWidth: 440 }}>
        <Dialog.Title>Install {item?.kind}</Dialog.Title>
        <Flex direction="column" gap="3">
          <Flex direction="column" gap="1">
            <Text size="3" weight="bold">
              {item?.name}
            </Text>
            <Text size="2" color="gray">
              {item?.description && item.description.length > 0
                ? item.description
                : "No description"}
            </Text>
            {item?.kind === "subagent" && (
              <Text size="1" color="gray">
                Installs as editable Refact YAML under `.refact/subagents` or
                your global config.
              </Text>
            )}
          </Flex>

          <Flex gap="2" wrap="wrap">
            <Badge variant="soft">{item?.source_label}</Badge>
            {item?.tags.map((tag) => (
              <Badge key={tag} variant="soft" color="gray">
                {tag}
              </Badge>
            ))}
          </Flex>

          <Flex direction="column" gap="1" className={styles.installScope}>
            <Text size="1">Install to:</Text>
            {hasProjectRoot ? (
              <SegmentedControl.Root
                size="1"
                value={scope}
                onValueChange={(value) => setScope(value as "local" | "global")}
              >
                <SegmentedControl.Item value="global">
                  <Flex align="center" gap="1">
                    <GlobeIcon width={12} height={12} />
                    Global
                  </Flex>
                </SegmentedControl.Item>
                <SegmentedControl.Item value="local">
                  <Flex align="center" gap="1">
                    <FileIcon width={12} height={12} />
                    Project
                  </Flex>
                </SegmentedControl.Item>
              </SegmentedControl.Root>
            ) : (
              <Badge size="1" color="blue" variant="soft">
                <Flex align="center" gap="1">
                  <GlobeIcon width={10} height={10} />
                  Global only (no project open)
                </Flex>
              </Badge>
            )}
          </Flex>

          {item?.params && item.params.length > 0 && (
            <Flex direction="column" gap="2">
              <Text size="1" weight="bold">
                Parameters
              </Text>
              {item.params.map((param) => (
                <Flex key={param.name} direction="column" gap="1">
                  <Text size="1">
                    {param.label || param.name}
                    {param.required && (
                      <Text color="red" size="1">
                        {" "}
                        *
                      </Text>
                    )}
                  </Text>
                  <TextField.Root
                    size="1"
                    placeholder={param.default ?? `Enter ${param.name}…`}
                    value={paramValues[param.name] ?? ""}
                    onChange={(e) =>
                      setParamValues((prev) => ({
                        ...prev,
                        [param.name]: e.target.value,
                      }))
                    }
                  />
                </Flex>
              ))}
            </Flex>
          )}

          {isConflict && (
            <Callout.Root color="amber" size="1">
              <Callout.Icon>
                <ExclamationTriangleIcon />
              </Callout.Icon>
              <Callout.Text>
                Already installed in this scope. Click{" "}
                <strong>Overwrite</strong> to replace it, or switch the scope
                above.
              </Callout.Text>
            </Callout.Root>
          )}

          {error && !isConflict && (
            <Text size="2" color="red">
              {error}
            </Text>
          )}
        </Flex>

        <Flex gap="3" mt="4" justify="end">
          <Dialog.Close>
            <Button variant="soft" color="gray">
              Cancel
            </Button>
          </Dialog.Close>
          {isConflict ? (
            <Button
              color="amber"
              onClick={() => handleInstallClick(true)}
              disabled={!item || isInstalling}
            >
              {isInstalling ? "Overwriting…" : "Overwrite"}
            </Button>
          ) : (
            <Button
              onClick={() => handleInstallClick(false)}
              disabled={!item || isInstalling}
            >
              {isInstalling ? "Installing…" : "Install"}
            </Button>
          )}
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};

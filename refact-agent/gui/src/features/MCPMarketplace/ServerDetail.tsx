import React, { useState } from "react";
import {
  Badge,
  Box,
  Button,
  Flex,
  Text,
  TextField,
  Heading,
  Callout,
} from "@radix-ui/themes";
import {
  ArrowLeftIcon,
  CheckIcon,
  ExternalLinkIcon,
  InfoCircledIcon,
} from "@radix-ui/react-icons";
import type { MCPServer } from "../../services/refact/mcpMarketplace";
import {
  useInstallServerMutation,
  useGetInstalledServersQuery,
} from "../../services/refact/mcpMarketplace";
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
  const [envValues, setEnvValues] = useState<Record<string, string | undefined>>(
    Object.fromEntries(Object.entries(defaultEnv).map(([k, v]) => [k, v])),
  );

  const [installServer, { isLoading, isSuccess, error }] =
    useInstallServerMutation();
  const { data: installedData } = useGetInstalledServersQuery(undefined);

  const isInstalled =
    installedData?.installed.some((s) => s.id === server.id) ?? false;

  const handleInstall = async () => {
    const definedEnv = Object.fromEntries(
      Object.entries(envValues).filter((e): e is [string, string] => e[1] !== undefined),
    );
    const configOverrides =
      Object.keys(definedEnv).length > 0 ? { env: definedEnv } : undefined;
    await installServer({ server_id: server.id, source_id: server.source_id, config_overrides: configOverrides });
  };

  const errorMessage =
    error && "data" in error
      ? String(error.data)
      : error
        ? "Installation failed"
        : null;

  return (
    <Flex direction="column" gap="4" style={{ height: "100%" }}>
      <Flex align="center" gap="2">
        <Button variant="ghost" size="1" onClick={onBack}>
          <ArrowLeftIcon />
          Back
        </Button>
      </Flex>

      <Flex align="center" gap="3">
        <Box className={styles.serverIconPlaceholderLarge}>
          <Text size="6" weight="bold">
            {server.name.charAt(0).toUpperCase()}
          </Text>
        </Box>
        <Flex direction="column" gap="1">
          <Heading size="4">{server.name}</Heading>
          <Text size="2" color="gray">
            by {server.publisher}
          </Text>
          <Flex gap="2" align="center">
            <Badge color="blue" variant="soft">
              {server.transport}
            </Badge>
            {server.homepage && (
              <Button
                size="1"
                variant="ghost"
                asChild
              >
                <a href={server.homepage} target="_blank" rel="noopener noreferrer">
                  <ExternalLinkIcon />
                  Homepage
                </a>
              </Button>
            )}
          </Flex>
        </Flex>
      </Flex>

      <Text size="2">{server.description}</Text>

      {server.tags.length > 0 && (
        <Flex gap="2" wrap="wrap">
          {server.tags.map((tag) => (
            <Badge key={tag} variant="soft" color="gray">
              {tag}
            </Badge>
          ))}
        </Flex>
      )}

      {Object.keys(defaultEnv).length > 0 && (
        <Flex direction="column" gap="2">
          <Text size="2" weight="bold">
            Configuration
          </Text>
          {Object.keys(defaultEnv).map((key) => (
            <Flex key={key} direction="column" gap="1">
              <Text size="1" color="gray">
                {key}
              </Text>
              <TextField.Root
                size="1"
                value={envValues[key] ?? ""}
                onChange={(e) =>
                  setEnvValues((prev) => ({ ...prev, [key]: e.target.value }))
                }
                placeholder={defaultEnv[key]}
              />
            </Flex>
          ))}
        </Flex>
      )}

      {errorMessage && (
        <Callout.Root color="red" size="1">
          <Callout.Icon>
            <InfoCircledIcon />
          </Callout.Icon>
          <Callout.Text>{errorMessage}</Callout.Text>
        </Callout.Root>
      )}

      {isSuccess && (
        <Callout.Root color="green" size="1">
          <Callout.Icon>
            <CheckIcon />
          </Callout.Icon>
          <Callout.Text>Server installed successfully!</Callout.Text>
        </Callout.Root>
      )}

      {isInstalled && !isSuccess && (
        <Callout.Root color="green" size="1">
          <Callout.Icon>
            <CheckIcon />
          </Callout.Icon>
          <Callout.Text>Already installed</Callout.Text>
        </Callout.Root>
      )}

      {!isInstalled && (
        <Button
          onClick={() => void handleInstall()}
          disabled={isLoading}
          style={{ alignSelf: "flex-start" }}
        >
          {isLoading ? "Installing…" : "Install"}
        </Button>
      )}
    </Flex>
  );
};

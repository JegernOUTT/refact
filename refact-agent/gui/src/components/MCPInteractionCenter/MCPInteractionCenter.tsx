import { useCallback, useEffect, useMemo, useState } from "react";
import { Bot, ClipboardList } from "lucide-react";

import {
  useListMcpInteractionsQuery,
  useRespondMcpInteractionMutation,
  type MCPInteraction,
  type MCPInteractionAction,
} from "../../services/refact/mcpInteractions";
import { useOpenUrl } from "../../hooks/useOpenUrl";
import { Badge, Button, Dialog, Flex, Icon, Surface, Text } from "../ui";
import { ElicitationForm } from "./ElicitationForm";
import styles from "./MCPInteractionCenter.module.css";

function formatRemaining(timeoutAtMs: number, nowMs: number) {
  const remainingSeconds = Math.max(0, Math.ceil((timeoutAtMs - nowMs) / 1000));
  const minutes = Math.floor(remainingSeconds / 60);
  const seconds = remainingSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

function useNow(ticking: boolean) {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!ticking) return;

    const interval = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(interval);
  }, [ticking]);

  return now;
}

function getInteractionLabel(interaction: MCPInteraction) {
  return interaction.kind === "elicitation"
    ? "Server needs your input"
    : "Server requests AI sampling";
}

export function MCPInteractionCenter() {
  const { data } = useListMcpInteractionsQuery(undefined, {
    pollingInterval: 3000,
  });
  const [respond, { isLoading }] = useRespondMcpInteractionMutation();
  const openUrl = useOpenUrl();
  const [openedUrlId, setOpenedUrlId] = useState<string | null>(null);

  const interactions = data?.interactions ?? [];
  const interaction = interactions.at(0);
  const now = useNow(interactions.length > 0);

  useEffect(() => {
    setOpenedUrlId(null);
  }, [interaction?.id]);

  const respondToInteraction = useCallback(
    async (action: MCPInteractionAction, content?: Record<string, unknown>) => {
      if (!interaction) return;

      try {
        await respond({ id: interaction.id, action, content }).unwrap();
      } catch {
        // Expired or handled elsewhere: polling will remove it.
      } finally {
        setOpenedUrlId(null);
      }
    },
    [interaction, respond],
  );

  const handleOpenUrl = useCallback(() => {
    if (!interaction?.url) return;
    openUrl(interaction.url);
    setOpenedUrlId(interaction.id);
  }, [interaction, openUrl]);

  const title = useMemo(() => {
    if (!interaction) return "MCP interaction";
    return `${interaction.server_name}: ${getInteractionLabel(interaction)}`;
  }, [interaction]);

  if (!interaction) return null;

  const label = getInteractionLabel(interaction);

  return (
    <Dialog
      open={true}
      onOpenChange={(open) => {
        if (!open) void respondToInteraction("decline");
      }}
    >
      <Dialog.Content maxWidth="min(92vw, 560px)">
        <Flex className={styles.header} align="start" gap="3" justify="between">
          <Flex align="start" gap="3">
            <Surface className={styles.iconWrap} variant="glass" radius="pill">
              <Icon
                icon={interaction.kind === "elicitation" ? ClipboardList : Bot}
                tone="accent"
                size="lg"
              />
            </Surface>
            <Flex direction="column" gap="1">
              <Dialog.Title className={styles.title}>{title}</Dialog.Title>
              <Text as="p" color="gray" size="2">
                {label}
              </Text>
            </Flex>
          </Flex>
          <Flex align="center" gap="2">
            {interactions.length > 1 ? (
              <Badge tone="accent" variant="glass">
                1 of {interactions.length}
              </Badge>
            ) : null}
            <Badge tone="muted" variant="outline" aria-label="Time remaining">
              {formatRemaining(interaction.timeout_at_ms, now)}
            </Badge>
          </Flex>
        </Flex>

        <Dialog.Description className={styles.srOnly}>
          Model Context Protocol server interaction
        </Dialog.Description>

        {interaction.kind === "elicitation" && interaction.requested_schema ? (
          <Flex direction="column" gap="3">
            {interaction.message ? (
              <Text as="p" className={styles.message}>
                {interaction.message}
              </Text>
            ) : null}
            <ElicitationForm
              key={interaction.id}
              schema={interaction.requested_schema}
              disabled={isLoading}
              onCancel={() => void respondToInteraction("cancel")}
              onDecline={() => void respondToInteraction("decline")}
              onSubmit={(content) =>
                void respondToInteraction("accept", content)
              }
            />
          </Flex>
        ) : null}

        {interaction.kind === "elicitation" &&
        interaction.url &&
        !interaction.requested_schema ? (
          <Flex direction="column" gap="3">
            {interaction.message ? (
              <Text as="p" className={styles.message}>
                {interaction.message}
              </Text>
            ) : null}
            <Surface className={styles.urlCard} variant="glass" radius="card">
              <Text as="p" className={styles.urlText}>
                {interaction.url}
              </Text>
            </Surface>
            <Flex
              className={styles.actions}
              gap="2"
              justify="between"
              wrap="wrap"
            >
              <Button
                type="button"
                variant="plain"
                disabled={isLoading}
                onClick={() => void respondToInteraction("cancel")}
              >
                Cancel operation
              </Button>
              <Flex gap="2" justify="end" wrap="wrap">
                <Button
                  type="button"
                  variant="ghost"
                  disabled={isLoading}
                  onClick={() => void respondToInteraction("decline")}
                >
                  Decline
                </Button>
                {openedUrlId === interaction.id ? (
                  <Button
                    type="button"
                    variant="primary"
                    disabled={isLoading}
                    onClick={() => void respondToInteraction("accept")}
                  >
                    I&apos;ve completed it
                  </Button>
                ) : (
                  <Button
                    type="button"
                    variant="primary"
                    disabled={isLoading}
                    onClick={handleOpenUrl}
                  >
                    Open in browser
                  </Button>
                )}
              </Flex>
            </Flex>
          </Flex>
        ) : null}

        {interaction.kind === "sampling_approval" ? (
          <Flex direction="column" gap="3">
            <Text as="p" className={styles.message}>
              The server wants to run an AI completion through your configured
              model.
            </Text>
            {interaction.preview ? (
              <Surface className={styles.preview} variant="glass" radius="card">
                <Text as="div" className={styles.previewText}>
                  {interaction.preview}
                </Text>
              </Surface>
            ) : null}
            <Flex gap="2" wrap="wrap">
              {interaction.message_count !== undefined ? (
                <Text color="gray" size="1">
                  Messages: {interaction.message_count}
                </Text>
              ) : null}
              {interaction.max_tokens !== undefined ? (
                <Text color="gray" size="1">
                  Max tokens: {interaction.max_tokens}
                </Text>
              ) : null}
            </Flex>
            <Flex className={styles.actions} gap="2" justify="end" wrap="wrap">
              <Button
                type="button"
                variant="ghost"
                disabled={isLoading}
                onClick={() => void respondToInteraction("decline")}
              >
                Deny
              </Button>
              <Button
                type="button"
                variant="primary"
                disabled={isLoading}
                onClick={() => void respondToInteraction("accept")}
              >
                Allow for this session
              </Button>
            </Flex>
          </Flex>
        ) : null}
      </Dialog.Content>
    </Dialog>
  );
}

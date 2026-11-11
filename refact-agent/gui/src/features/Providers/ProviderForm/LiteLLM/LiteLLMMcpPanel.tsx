import React from "react";

import { Badge, Button, Surface } from "../../../../components/ui";
import { MCPOAuth } from "../../../../components/IntegrationsView/MCPServerView/MCPOAuth";
import { useAppDispatch } from "../../../../hooks";
import { useOpenUrl } from "../../../../hooks/useOpenUrl";
import { change } from "../../../Pages/pagesSlice";
import {
  useGetLiteLLMMcpQuery,
  useRescanLiteLLMMcpMutation,
  useSetLiteLLMMcpServerEnabledMutation,
  type LiteLLMMcpServer,
} from "../../../../services/refact/providers";
import {
  FEATURE_STATE_LABEL,
  featureStateExplanation,
  featureStateTone,
  formatWhen,
  isSafeExternalUrl,
  type LiteLLMTone,
} from "./litellmFormat";
import styles from "./LiteLLM.module.css";

const MCP_POLL_MS = 30_000;

function authBadge(server: LiteLLMMcpServer): {
  label: string;
  tone: LiteLLMTone;
} | null {
  switch (server.auth.state) {
    case "auth_required":
      return { label: "Auth required", tone: "warning" };
    case "missing_setup":
      return { label: "Missing setup", tone: "warning" };
    case "permission_denied":
      return { label: "Permission denied", tone: "danger" };
    case "unknown":
      return { label: "Auth unknown", tone: "muted" };
    default:
      return null;
  }
}

function connectionBadge(server: LiteLLMMcpServer): React.ReactNode {
  if (!server.enabled) return <Badge tone="muted">Disabled</Badge>;
  switch (server.connection) {
    case "connecting":
      return <Badge tone="accent">Connecting</Badge>;
    case "needs_auth":
      return <Badge tone="warning">Needs sign-in</Badge>;
    case "failed":
      return <Badge tone="danger">Connection failed</Badge>;
    case "reconfirm_required":
      return <Badge tone="warning">Reconfirm required</Badge>;
    case "connected":
      return <Badge tone="success">Connected</Badge>;
    default:
      return (
        <Badge tone={server.connected ? "success" : "accent"}>
          {server.connected ? "Connected" : "Enabled"}
        </Badge>
      );
  }
}

function describeAuth(server: LiteLLMMcpServer): string {
  switch (server.auth.state) {
    case "auth_required":
      return server.auth.kind === "oauth"
        ? "Sign in with OAuth through the gateway to use this server."
        : "This server requires authentication before it can be used.";
    case "missing_setup":
      return "This server needs credentials or variables to be set up in the LiteLLM gateway before it can be used.";
    case "permission_denied":
      return "Your key is not permitted to use this server.";
    case "unknown":
      return "Authentication status could not be determined.";
    default:
      return "";
  }
}

const ServerCard: React.FC<{
  providerName: string;
  server: LiteLLMMcpServer;
  stale: boolean;
}> = ({ providerName, server, stale }) => {
  const dispatch = useAppDispatch();
  const openUrl = useOpenUrl();
  const [setEnabled, { isLoading: isToggling }] =
    useSetLiteLLMMcpServerEnabledMutation();
  const [error, setError] = React.useState<string | null>(null);

  const denied = server.access === "denied";
  const badge = authBadge(server);
  const setupUrl = isSafeExternalUrl(server.auth.setup_url)
    ? server.auth.setup_url ?? null
    : null;

  const toggle = async (enabled: boolean) => {
    setError(null);
    const result = await setEnabled({
      providerName,
      serverId: server.server_id,
      enabled,
    });
    if ("error" in result) {
      setError(
        enabled
          ? "Couldn't enable this server. The previous setting was kept."
          : "Couldn't disable this server. The previous setting was kept.",
      );
    }
  };

  const openLocalSettings = () => {
    if (!server.integration_path) return;
    dispatch(
      change({
        name: "integrations page",
        integrationPath: server.integration_path,
      }),
    );
  };

  return (
    <li
      className={styles.card}
      data-testid={`litellm-mcp-${server.server_id}`}
      aria-label={server.display_name}
    >
      <div className={styles.serverHead}>
        <div className={styles.serverName}>{server.display_name}</div>
        <div className={styles.badges}>
          {server.removed ? (
            <Badge tone="danger">Removed from gateway</Badge>
          ) : null}
          {connectionBadge(server)}
          {denied ? <Badge tone="danger">Access denied</Badge> : null}
          {server.access === "unknown" ? (
            <Badge tone="muted">Access unknown</Badge>
          ) : null}
          {badge ? <Badge tone={badge.tone}>{badge.label}</Badge> : null}
          <Badge
            tone={
              server.health === "healthy"
                ? "success"
                : server.health === "unhealthy"
                  ? "danger"
                  : "muted"
            }
          >
            {server.health === "healthy"
              ? "Healthy"
              : server.health === "unhealthy"
                ? "Unhealthy"
                : "Health unknown"}
          </Badge>
          {stale ? <Badge tone="warning">Stale</Badge> : null}
        </div>
      </div>
      {server.description ? (
        <div className={styles.meta}>{server.description}</div>
      ) : null}
      {describeAuth(server) ? (
        <div className={styles.notice}>{describeAuth(server)}</div>
      ) : null}
      {server.auth.status_check_state &&
      server.auth.status_check_state !== "ok" ? (
        <div className={styles.notice} role="status">
          Gateway credential status:{" "}
          {FEATURE_STATE_LABEL[server.auth.status_check_state]}. This does not
          determine whether tool access is permitted.
        </div>
      ) : null}
      {server.auth.kind === "oauth" ? (
        <div className={styles.meta}>
          LiteLLM may share upstream credentials across keys belonging to the
          same gateway user. Use the browser account associated with this
          provider; Refact cannot verify that the browser user matches the key
          owner. Logout clears only this local session, not the shared gateway
          credential.
        </div>
      ) : null}
      {server.connection === "reconfirm_required" ? (
        <div className={styles.notice} role="status">
          The gateway endpoint or credential identity changed. Disable and
          re-enable this server to confirm it before it is used again.
        </div>
      ) : null}
      {typeof server.tools_count === "number" ? (
        <div className={styles.meta}>{server.tools_count} tools</div>
      ) : null}

      {server.enabled &&
      server.auth.kind === "oauth" &&
      server.integration_path &&
      !denied ? (
        <MCPOAuth configPath={server.integration_path} />
      ) : null}

      <div className={styles.actions}>
        {server.enabled ? (
          <Button
            size="1"
            variant="soft"
            loading={isToggling}
            disabled={isToggling}
            onClick={() => void toggle(false)}
          >
            Disable
          </Button>
        ) : (
          <Button
            size="1"
            variant="primary"
            loading={isToggling}
            disabled={isToggling || denied || server.removed === true}
            onClick={() => void toggle(true)}
          >
            Enable
          </Button>
        )}
        {setupUrl ? (
          <Button size="1" variant="ghost" onClick={() => openUrl(setupUrl)}>
            Open gateway setup
          </Button>
        ) : null}
        {server.integration_path ? (
          <Button size="1" variant="ghost" onClick={openLocalSettings}>
            Approvals &amp; logs
          </Button>
        ) : null}
      </div>
      {!server.enabled ? (
        <div className={styles.meta}>
          Enabling adds this server to Refact&apos;s MCP tools. Tool calls still
          follow your local confirmation and privacy rules.
        </div>
      ) : null}
      {error ? (
        <div className={styles.noticeDanger} role="alert">
          {error}
        </div>
      ) : null}
    </li>
  );
};

export const LiteLLMMcpPanel: React.FC<{ providerName: string }> = ({
  providerName,
}) => {
  const { currentData, isError, isFetching, isLoading, refetch } =
    useGetLiteLLMMcpQuery(
      { providerName },
      { pollingInterval: MCP_POLL_MS, skipPollingIfUnfocused: true },
    );
  const [rescan, { isLoading: isRescanning }] = useRescanLiteLLMMcpMutation();
  const [rescanError, setRescanError] = React.useState<string | null>(null);
  const fetchedAt = formatWhen(currentData?.fetched_at);

  const handleRescan = async () => {
    setRescanError(null);
    const result = await rescan({ providerName });
    if ("error" in result) setRescanError("Rescan failed.");
  };

  return (
    <Surface className={styles.section} variant="glass">
      <div className={styles.header}>
        <div>
          <div className={styles.title}>MCP servers</div>
          <div className={styles.meta}>
            Servers hosted by this LiteLLM gateway and visible to this
            provider&apos;s key. Nothing is enabled automatically.
            {fetchedAt ? ` Updated ${fetchedAt}.` : ""}
          </div>
          <div className={styles.meta}>
            OAuth uses a local callback by default and requires the browser to
            reach this engine on the same machine or through port forwarding.
            For a remote engine, configure its externally reachable MCP OAuth
            callback URL and matching ingress route in the provider settings.
          </div>
        </div>
        <div className={styles.badges}>
          {isFetching && !isLoading ? (
            <Badge tone="accent">Refreshing</Badge>
          ) : null}
          {currentData?.stale ? (
            <Badge tone="warning">Inventory stale</Badge>
          ) : null}
          {currentData && currentData.state !== "ok" ? (
            <Badge tone={featureStateTone(currentData.state)}>
              {FEATURE_STATE_LABEL[currentData.state]}
            </Badge>
          ) : null}
          <Button
            size="1"
            variant="ghost"
            loading={isRescanning}
            disabled={isRescanning || isFetching}
            onClick={() => void handleRescan()}
          >
            Rescan
          </Button>
          <Button
            size="1"
            variant="ghost"
            disabled={isFetching}
            onClick={() => void refetch()}
          >
            Refresh
          </Button>
        </div>
      </div>

      {isLoading ? <div className={styles.meta}>Loading servers…</div> : null}
      {isError ? (
        <div className={styles.noticeDanger} role="alert">
          {currentData
            ? "Failed to refresh the server list; showing the last known inventory."
            : "Failed to load MCP servers."}
        </div>
      ) : null}
      {rescanError ? (
        <div className={styles.noticeDanger} role="alert">
          {rescanError}
        </div>
      ) : null}
      {currentData?.reconfirm_required ? (
        <div className={styles.notice} role="status">
          The gateway identity changed since these servers were enabled.
          Re-enable each server you want to keep using.
        </div>
      ) : null}
      {currentData && currentData.state !== "ok" ? (
        <div className={styles.notice} role="status">
          {currentData.message ??
            featureStateExplanation(currentData.state, "MCP servers")}
        </div>
      ) : null}
      {currentData?.state === "ok" && currentData.servers.length === 0 ? (
        <div className={styles.notice}>
          No MCP servers are available to this key.
        </div>
      ) : null}

      {currentData && currentData.servers.length > 0 ? (
        <ul className={styles.serverList}>
          {currentData.servers.map((server) => (
            <ServerCard
              key={server.server_id}
              providerName={providerName}
              server={server}
              stale={currentData.stale === true}
            />
          ))}
        </ul>
      ) : null}
    </Surface>
  );
};

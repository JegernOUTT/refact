import React, { useEffect, useState } from "react";
import { integrationsApi } from "../../../services/refact/integrations";
import { useOpenUrl } from "../../../hooks/useOpenUrl";
import {
  Badge,
  Button,
  FieldTextarea,
  Flex,
  Spinner,
  Surface,
  Text,
} from "../../ui";
import styles from "./MCPOAuth.module.css";

type MCPOAuthProps = {
  configPath: string;
};

export const MCPOAuth: React.FC<MCPOAuthProps> = ({ configPath }) => {
  const openUrl = useOpenUrl();

  const [pollingInterval, setPollingInterval] = useState(3000);

  const { data: status, isLoading } = integrationsApi.useMcpOauthStatusQuery(
    configPath,
    { pollingInterval, skip: !configPath },
  );
  const [oauthStart] = integrationsApi.useMcpOauthStartMutation();
  const [oauthExchange] = integrationsApi.useMcpOauthExchangeMutation();
  const [oauthLogout] = integrationsApi.useMcpOauthLogoutMutation();
  const [oauthCancel] = integrationsApi.useMcpOauthCancelMutation();

  const [sessionId, setSessionId] = useState<string | null>(null);
  const [authorizeUrl, setAuthorizeUrl] = useState<string | null>(null);
  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [isWorking, setIsWorking] = useState(false);
  const [waitingForCallback, setWaitingForCallback] = useState(false);

  useEffect(() => {
    if (waitingForCallback && status?.authenticated) {
      setWaitingForCallback(false);
      setSessionId(null);
      setAuthorizeUrl(null);
    }
  }, [waitingForCallback, status?.authenticated]);

  useEffect(() => {
    const shouldPoll =
      waitingForCallback ||
      (status !== undefined &&
        !status.authenticated &&
        (status.auth_type === "oauth2_pkce" ||
          (status.needs_login && status.oauth_available)));
    setPollingInterval(shouldPoll ? 3000 : 0);
  }, [waitingForCallback, status]);

  const handleStartOAuth = async () => {
    setError(null);
    setIsWorking(true);
    try {
      const result = await oauthStart({ config_path: configPath }).unwrap();
      setSessionId(result.session_id);
      setAuthorizeUrl(result.authorize_url);
      openUrl(result.authorize_url);
      setWaitingForCallback(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to start OAuth");
    } finally {
      setIsWorking(false);
    }
  };

  const handleExchangeCode = async () => {
    if (!sessionId || !code.trim()) return;
    setError(null);
    setIsWorking(true);
    try {
      await oauthExchange({
        session_id: sessionId,
        code: code.trim(),
      }).unwrap();
      setSessionId(null);
      setAuthorizeUrl(null);
      setCode("");
      setWaitingForCallback(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to exchange code");
    } finally {
      setIsWorking(false);
    }
  };

  const handleLogout = async () => {
    setError(null);
    setIsWorking(true);
    try {
      await oauthLogout({ config_path: configPath }).unwrap();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to logout");
    } finally {
      setIsWorking(false);
    }
  };

  const handleCancel = async () => {
    if (sessionId) {
      try {
        await oauthCancel({ session_id: sessionId }).unwrap();
      } catch {
        setError(null);
      }
    }
    setSessionId(null);
    setAuthorizeUrl(null);
    setCode("");
    setWaitingForCallback(false);
  };

  if (isLoading) return null;
  if (!status) return null;
  const oauthConfigured = status.auth_type === "oauth2_pkce";
  const oauthOffered = status.needs_login && status.oauth_available;
  if (!oauthConfigured && !oauthOffered) return null;

  const expiresAtMs = Number.isFinite(status.expires_at)
    ? status.expires_at
    : typeof status.expires_at === "string"
      ? Date.parse(status.expires_at)
      : null;
  const isExpired =
    expiresAtMs != null && expiresAtMs !== 0 && expiresAtMs < Date.now();
  const expiryDate =
    expiresAtMs != null && !Number.isNaN(expiresAtMs)
      ? new Date(expiresAtMs)
      : null;

  if (status.authenticated) {
    return (
      <Surface
        animated="rise"
        className={styles.container}
        radius="card"
        variant="glass"
      >
        <Flex direction="column" gap="2">
          <Flex align="center" justify="between" gap="2" wrap="wrap">
            <Flex align="center" gap="2" wrap="wrap">
              <Badge aria-label="Authenticated" tone="success">
                Authenticated
              </Badge>
              {expiryDate && (
                <Text size="1" color="gray">
                  Expires: {expiryDate.toLocaleString()}
                </Text>
              )}
            </Flex>
            <Button
              variant="danger"
              size="sm"
              disabled={isWorking}
              onClick={() => void handleLogout()}
            >
              Logout
            </Button>
          </Flex>
          {error && (
            <Text as="p" size="1" color="red">
              {error}
            </Text>
          )}
        </Flex>
      </Surface>
    );
  }

  if (waitingForCallback && sessionId && authorizeUrl) {
    return (
      <Surface
        animated="rise"
        className={styles.container}
        radius="card"
        variant="glass"
      >
        <Flex direction="column" gap="2">
          <Flex align="center" gap="2">
            <Spinner size="sm" />
            <Text size="2" weight="medium">
              Waiting for authorization...
            </Text>
          </Flex>
          <Text as="p" size="1" color="gray">
            Complete the login in the browser window that opened.
          </Text>
          <Text size="2" weight="medium">
            Or enter the authorization code manually:
          </Text>
          <FieldTextarea
            placeholder="Paste authorization code here..."
            value={code}
            onChange={setCode}
            rows={2}
            aria-label="Authorization code"
          />
          <Flex gap="2" wrap="wrap">
            <Button
              size="md"
              variant="primary"
              disabled={isWorking || !code.trim()}
              onClick={() => void handleExchangeCode()}
            >
              {isWorking ? "Submitting..." : "Submit Code"}
            </Button>
            <Button
              size="md"
              variant="ghost"
              onClick={() => void handleCancel()}
            >
              Cancel
            </Button>
          </Flex>
          <Flex gap="2" align="center">
            <Text size="1" color="gray">
              Browser didn&apos;t open?{" "}
              <a
                href="#"
                className={styles.accentLink}
                onClick={(e) => {
                  e.preventDefault();
                  openUrl(authorizeUrl);
                }}
              >
                Click here
              </a>
            </Text>
          </Flex>
          {error && (
            <Text as="p" size="1" color="red">
              {error}
            </Text>
          )}
        </Flex>
      </Surface>
    );
  }

  return (
    <Surface
      animated="rise"
      className={styles.container}
      radius="card"
      variant="glass"
    >
      <Flex direction="column" gap="2">
        {!oauthConfigured && oauthOffered && (
          <Text as="p" size="1" color="gray">
            This server requires authentication. Login with OAuth to connect.
          </Text>
        )}
        <Flex align="center" justify="between" gap="2" wrap="wrap">
          <Flex align="center" gap="2">
            {isExpired ? (
              <Badge tone="warning">Session expired</Badge>
            ) : (
              <Badge tone="muted">Not authenticated</Badge>
            )}
          </Flex>
          <Button
            size="md"
            variant="primary"
            disabled={isWorking}
            onClick={() => void handleStartOAuth()}
          >
            {isWorking ? "Starting..." : "Login with OAuth"}
          </Button>
        </Flex>
        {isExpired && (
          <Text as="p" size="1" color="gray">
            Session expired, please re-login
          </Text>
        )}
        {error && (
          <Text as="p" size="1" color="red">
            {error}
          </Text>
        )}
      </Flex>
    </Surface>
  );
};

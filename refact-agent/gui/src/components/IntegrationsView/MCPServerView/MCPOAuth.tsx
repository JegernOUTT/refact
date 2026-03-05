import React, { useEffect, useState } from "react";
import { Badge, Button, Flex, Spinner, Text, TextArea } from "@radix-ui/themes";
import { integrationsApi } from "../../../services/refact/integrations";
import { useOpenUrl } from "../../../hooks/useOpenUrl";
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
      (!status?.authenticated && status?.auth_type === "oauth2_pkce");
    setPollingInterval(shouldPoll ? 3000 : status?.authenticated ? 0 : 0);
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
        // ignore error, clean up locally
      }
    }
    setSessionId(null);
    setAuthorizeUrl(null);
    setCode("");
    setWaitingForCallback(false);
  };

  if (isLoading) return null;
  if (!status || status.auth_type !== "oauth2_pkce") return null;

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
      <div className={styles.container}>
        <Flex direction="column" gap="2">
          <Flex align="center" justify="between">
            <Flex align="center" gap="2">
              <Badge aria-label="Authenticated">Authenticated</Badge>
              {expiryDate && (
                <Text size="1" color="gray">
                  Expires: {expiryDate.toLocaleString()}
                </Text>
              )}
            </Flex>
            <Button
              variant="ghost"
              color="red"
              size="1"
              disabled={isWorking}
              onClick={() => void handleLogout()}
            >
              Logout
            </Button>
          </Flex>
          {error && (
            <Text size="1" color="red">
              {error}
            </Text>
          )}
        </Flex>
      </div>
    );
  }

  if (waitingForCallback && sessionId && authorizeUrl) {
    return (
      <div className={styles.container}>
        <Flex direction="column" gap="2">
          <Flex align="center" gap="2">
            <Spinner size="1" />
            <Text size="2" weight="medium">
              Waiting for authorization...
            </Text>
          </Flex>
          <Text size="1" color="gray">
            Complete the login in the browser window that opened.
          </Text>
          <Text size="2" weight="medium">
            Or enter the authorization code manually:
          </Text>
          <TextArea
            placeholder="Paste authorization code here..."
            value={code}
            onChange={(e) => setCode(e.target.value)}
            rows={2}
            aria-label="Authorization code"
          />
          <Flex gap="2">
            <Button
              size="2"
              variant="solid"
              disabled={isWorking || !code.trim()}
              onClick={() => void handleExchangeCode()}
            >
              {isWorking ? "Submitting..." : "Submit Code"}
            </Button>
            <Button
              size="2"
              variant="ghost"
              color="gray"
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
            <Text size="1" color="red">
              {error}
            </Text>
          )}
        </Flex>
      </div>
    );
  }

  return (
    <div className={styles.container}>
      <Flex direction="column" gap="2">
        <Flex align="center" justify="between">
          <Flex align="center" gap="2">
            {isExpired ? (
              <Badge>Session expired</Badge>
            ) : (
              <Badge>Not authenticated</Badge>
            )}
          </Flex>
          <Button
            size="2"
            variant="solid"
            disabled={isWorking}
            onClick={() => void handleStartOAuth()}
          >
            {isWorking ? "Starting..." : "Login with OAuth"}
          </Button>
        </Flex>
        {isExpired && (
          <Text size="1" color="gray">
            Session expired, please re-login
          </Text>
        )}
        {error && (
          <Text size="1" color="red">
            {error}
          </Text>
        )}
      </Flex>
    </div>
  );
};

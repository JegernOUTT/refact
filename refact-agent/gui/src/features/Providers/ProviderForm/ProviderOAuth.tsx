import React, { useCallback, useEffect, useRef, useState } from "react";
import { Button, Flex, Text, TextField } from "@radix-ui/themes";
import {
  useOauthStartMutation,
  useOauthExchangeMutation,
  useOauthLogoutMutation,
  providersApi,
} from "../../../services/refact";
import { useAppDispatch } from "../../../hooks";
import { useOpenUrl } from "../../../hooks/useOpenUrl";

const PROVIDERS_WITH_AUTO_CALLBACK = ["openai_codex"];

const PROVIDER_LOGIN_LABELS: Record<string, string> = {
  claude_code: "Login with Anthropic",
  openai_codex: "Login with OpenAI",
};

type ProviderOAuthProps = {
  providerName: string;
  oauthConnected: boolean;
  authStatus: string;
};

export const ProviderOAuth: React.FC<ProviderOAuthProps> = ({
  providerName,
  oauthConnected,
  authStatus,
}) => {
  const dispatch = useAppDispatch();
  const openUrl = useOpenUrl();
  const [oauthStart] = useOauthStartMutation();
  const [oauthExchange] = useOauthExchangeMutation();
  const [oauthLogout] = useOauthLogoutMutation();

  const [sessionId, setSessionId] = useState<string | null>(null);
  const [authorizeUrl, setAuthorizeUrl] = useState<string | null>(null);
  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [waitingForCallback, setWaitingForCallback] = useState(false);
  const pollTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const isAutoCallback = PROVIDERS_WITH_AUTO_CALLBACK.includes(providerName);
  const loginLabel = PROVIDER_LOGIN_LABELS[providerName] || "Login";

  const invalidateProvider = useCallback(() => {
    dispatch(
      providersApi.util.invalidateTags([
        { type: "PROVIDER", id: providerName },
        { type: "PROVIDERS", id: "LIST" },
        { type: "AVAILABLE_MODELS", id: providerName },
      ]),
    );
  }, [dispatch, providerName]);

  useEffect(() => {
    return () => {
      if (pollTimerRef.current) {
        clearInterval(pollTimerRef.current);
      }
    };
  }, []);

  const handleStartOAuth = async () => {
    setError(null);
    setIsLoading(true);
    try {
      const result = await oauthStart({ providerName, mode: "max" }).unwrap();
      setSessionId(result.session_id);
      setAuthorizeUrl(result.authorize_url);
      openUrl(result.authorize_url);

      if (isAutoCallback) {
        setWaitingForCallback(true);
        pollTimerRef.current = setInterval(() => {
          invalidateProvider();
        }, 2000);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to start OAuth");
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    if (waitingForCallback && oauthConnected) {
      setWaitingForCallback(false);
      setSessionId(null);
      setAuthorizeUrl(null);
      if (pollTimerRef.current) {
        clearInterval(pollTimerRef.current);
        pollTimerRef.current = null;
      }
    }
  }, [waitingForCallback, oauthConnected]);

  // If backend updated auth_status to a terminal error while we were polling,
  // stop waiting and let the user see the status.
  useEffect(() => {
    if (!waitingForCallback) return;
    if (!authStatus) return;
    if (/failed|error|unavailable|missing/i.test(authStatus)) {
      setWaitingForCallback(false);
      if (pollTimerRef.current) {
        clearInterval(pollTimerRef.current);
        pollTimerRef.current = null;
      }
    }
  }, [waitingForCallback, authStatus]);

  const handleExchangeCode = async () => {
    if (!sessionId || !code.trim()) return;
    setError(null);
    setIsLoading(true);
    try {
      await oauthExchange({
        providerName,
        session_id: sessionId,
        code: code.trim(),
      }).unwrap();
      setSessionId(null);
      setAuthorizeUrl(null);
      setCode("");
      invalidateProvider();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to exchange code");
    } finally {
      setIsLoading(false);
    }
  };

  const handleLogout = async () => {
    setError(null);
    setIsLoading(true);
    try {
      await oauthLogout({ providerName }).unwrap();
      setSessionId(null);
      setAuthorizeUrl(null);
      setCode("");
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to logout");
    } finally {
      setIsLoading(false);
    }
  };

  const handleCancel = () => {
    setSessionId(null);
    setAuthorizeUrl(null);
    setCode("");
    setWaitingForCallback(false);
    if (pollTimerRef.current) {
      clearInterval(pollTimerRef.current);
      pollTimerRef.current = null;
    }
  };

  if (oauthConnected) {
    return (
      <Flex
        direction="column"
        gap="2"
        p="3"
        style={{
          border: "1px solid var(--gray-6)",
          borderRadius: "var(--radius-2)",
        }}
      >
        <Flex align="center" justify="between">
          <Flex align="center" gap="2">
            <Text size="2" weight="medium" color="green">
              ● Connected
            </Text>
            <Text size="1" color="gray">
              {authStatus}
            </Text>
          </Flex>
          <Button
            variant="ghost"
            color="red"
            size="1"
            disabled={isLoading}
            onClick={() => void handleLogout()}
          >
            Disconnect
          </Button>
        </Flex>
      </Flex>
    );
  }

  if (sessionId && authorizeUrl) {
    if (isAutoCallback && waitingForCallback) {
      return (
        <Flex
          direction="column"
          gap="2"
          p="3"
          style={{
            border: "1px solid var(--gray-6)",
            borderRadius: "var(--radius-2)",
          }}
        >
          <Text size="2" weight="medium">
            Waiting for authentication...
          </Text>
          <Text size="1" color="gray">
            Complete the login in the browser window that opened. This page will
            update automatically.
          </Text>
          <Flex gap="2" align="center">
            <Text size="1" color="gray">
              Browser didn&apos;t open?{" "}
              <a
                href="#"
                onClick={(e) => {
                  e.preventDefault();
                  if (authorizeUrl) openUrl(authorizeUrl);
                }}
                style={{ color: "var(--accent-9)" }}
              >
                Click here
              </a>
            </Text>
            <Button
              variant="ghost"
              size="1"
              color="gray"
              onClick={handleCancel}
            >
              Cancel
            </Button>
          </Flex>
          {error && (
            <Text size="1" color="red">
              {error}
            </Text>
          )}
        </Flex>
      );
    }

    return (
      <Flex
        direction="column"
        gap="2"
        p="3"
        style={{
          border: "1px solid var(--gray-6)",
          borderRadius: "var(--radius-2)",
        }}
      >
        <Text size="2" weight="medium">
          Paste the authorization code
        </Text>
        <Text size="1" color="gray">
          A browser window should have opened. Log in and copy the code shown on
          the page.
        </Text>
        <Flex gap="2">
          <TextField.Root
            style={{ flex: 1 }}
            placeholder="Paste code here..."
            value={code}
            onChange={(e) => setCode(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void handleExchangeCode();
            }}
          />
          <Button
            variant="solid"
            disabled={isLoading || !code.trim()}
            onClick={() => void handleExchangeCode()}
          >
            {isLoading ? "Connecting..." : "Connect"}
          </Button>
        </Flex>
        <Flex gap="2" align="center">
          <Text size="1" color="gray">
            Browser didn&apos;t open?{" "}
            <a
              href="#"
              onClick={(e) => {
                e.preventDefault();
                if (authorizeUrl) openUrl(authorizeUrl);
              }}
              style={{ color: "var(--accent-9)" }}
            >
              Click here
            </a>
          </Text>
          <Button variant="ghost" size="1" color="gray" onClick={handleCancel}>
            Cancel
          </Button>
        </Flex>
        {error && (
          <Text size="1" color="red">
            {error}
          </Text>
        )}
      </Flex>
    );
  }

  return (
    <Flex
      direction="column"
      gap="2"
      p="3"
      style={{
        border: "1px solid var(--gray-6)",
        borderRadius: "var(--radius-2)",
      }}
    >
      <Flex align="center" justify="between">
        <Text size="2" weight="medium">
          {loginLabel}
        </Text>
        <Button
          variant="solid"
          disabled={isLoading}
          onClick={() => void handleStartOAuth()}
        >
          {isLoading ? "Starting..." : "Login"}
        </Button>
      </Flex>
      {error && (
        <Text size="1" color="red">
          {error}
        </Text>
      )}
    </Flex>
  );
};

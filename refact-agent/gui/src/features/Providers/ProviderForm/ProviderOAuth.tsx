import React, { useCallback, useEffect, useRef, useState } from "react";

import {
  useOauthStartMutation,
  useOauthExchangeMutation,
  useOauthLogoutMutation,
  providersApi,
  capsApi,
} from "../../../services/refact";
import type {
  OAuthStartMode,
  OAuthStartResponse,
} from "../../../services/refact";
import { useAppDispatch } from "../../../hooks";
import { useOpenUrl } from "../../../hooks/useOpenUrl";
import { Button, FieldText, Surface } from "../../../components/ui";
import type { ClaudeLoginKind } from "./schemaConditions";

import styles from "./ProviderOAuth.module.css";

const PROVIDERS_WITH_AUTO_CALLBACK = [
  "claude_code",
  "openai_codex",
  "xai_oauth",
  "google_antigravity",
];

const PROVIDER_LOGIN_LABELS: Partial<Record<string, string>> = {
  claude_code: "Login with Anthropic",
  openai_codex: "Login with OpenAI",
  github_copilot: "Login with GitHub Copilot",
  xai_oauth: "Login with xAI",
  google_antigravity: "Login with Google",
};

const NATIVE_BACKEND_NAMES: Partial<Record<string, string>> = {
  bedrock: "AWS SSO",
  vertex: "Google Cloud",
  foundry: "Microsoft Azure",
};

const NATIVE_POLL_SECONDS = 2;

type ProviderOAuthProps = {
  providerName: string;
  baseProvider?: string;
  oauthConnected: boolean;
  authStatus: string;
  loginKind?: ClaudeLoginKind;
  backend?: string;
  readonly?: boolean;
};

type ActiveSession = {
  providerName: string;
  sessionId: string;
  native: boolean;
};

function getErrorMessage(error: unknown, fallback: string) {
  if (error instanceof Error) return error.message;
  if (typeof error === "object" && error !== null) {
    const record = error as Record<string, unknown>;
    const data = record.data;
    if (typeof data === "object" && data !== null) {
      const dataRecord = data as Record<string, unknown>;
      if (typeof dataRecord.detail === "string") return dataRecord.detail;
      if (typeof dataRecord.error === "string") return dataRecord.error;
    }
    if (typeof data === "string") return data;
    if (typeof record.error === "string") return record.error;
    if (typeof record.message === "string") return record.message;
  }
  return fallback;
}

function inferOAuthMode(
  providerName: string,
  response: OAuthStartResponse,
): OAuthStartMode {
  if (response.mode) return response.mode;
  if (response.user_code !== undefined || providerName === "github_copilot")
    return "device";
  if (PROVIDERS_WITH_AUTO_CALLBACK.includes(providerName)) return "callback";
  return "manual_code";
}

function nativeServiceName(backend: string | undefined): string {
  return (backend && NATIVE_BACKEND_NAMES[backend]) ?? "cloud provider";
}

function loginLabelFor(
  baseProvider: string,
  loginKind: ClaudeLoginKind | undefined,
  backend: string | undefined,
): string {
  if (loginKind === "console") return "Login with Anthropic Console";
  if (loginKind === "native") return `Login with ${nativeServiceName(backend)}`;
  return PROVIDER_LOGIN_LABELS[baseProvider] ?? "Login";
}

export const ProviderOAuth: React.FC<ProviderOAuthProps> = ({
  providerName,
  baseProvider = providerName,
  oauthConnected,
  authStatus,
  loginKind,
  backend,
  readonly = false,
}) => {
  const dispatch = useAppDispatch();
  const openUrl = useOpenUrl();
  const [oauthStart] = useOauthStartMutation();
  const [oauthExchange] = useOauthExchangeMutation();
  const [oauthLogout] = useOauthLogoutMutation();

  const [sessionId, setSessionId] = useState<string | null>(null);
  const [authorizeUrl, setAuthorizeUrl] = useState<string | null>(null);
  const [oauthMode, setOauthMode] = useState<OAuthStartMode | null>(null);
  const [userCode, setUserCode] = useState<string | null>(null);
  const [instructions, setInstructions] = useState<string | null>(null);
  const [pollIntervalSeconds, setPollIntervalSeconds] = useState<number | null>(
    null,
  );
  const [deviceStatus, setDeviceStatus] = useState<string | null>(null);
  const [nativeOutput, setNativeOutput] = useState<string | null>(null);
  const [completionMessage, setCompletionMessage] = useState<string | null>(
    null,
  );
  const [isDevicePolling, setIsDevicePolling] = useState(false);
  const [devicePollTick, setDevicePollTick] = useState(0);
  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [waitingForCallback, setWaitingForCallback] = useState(false);
  const callbackPollTimerRef = useRef<ReturnType<typeof setInterval> | null>(
    null,
  );
  const devicePollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const activeSessionRef = useRef<ActiveSession | null>(null);

  const loginLabel = loginLabelFor(baseProvider, loginKind, backend);
  const serviceName = loginLabel.replace("Login with ", "");
  const identity = `${providerName}\u0000${baseProvider}\u0000${
    loginKind ?? ""
  }\u0000${backend ?? ""}`;
  const identityRef = useRef(identity);
  identityRef.current = identity;
  const mountedRef = useRef(false);
  const surfaceClassName = `${styles.container} rf-stagger`;

  const clearCallbackPollTimer = useCallback(() => {
    if (callbackPollTimerRef.current) {
      clearInterval(callbackPollTimerRef.current);
      callbackPollTimerRef.current = null;
    }
  }, []);

  const clearDevicePollTimer = useCallback(() => {
    if (devicePollTimerRef.current) {
      clearTimeout(devicePollTimerRef.current);
      devicePollTimerRef.current = null;
    }
  }, []);

  const invalidateProvider = useCallback(() => {
    dispatch(
      providersApi.util.invalidateTags([
        { type: "PROVIDER", id: providerName },
        { type: "PROVIDERS", id: "LIST" },
        { type: "AVAILABLE_MODELS", id: providerName },
      ]),
    );
  }, [dispatch, providerName]);

  const invalidateProviderAndCaps = useCallback(() => {
    invalidateProvider();
    dispatch(capsApi.util.invalidateTags(["CAPS"]));
  }, [dispatch, invalidateProvider]);

  const resetOAuthState = useCallback(() => {
    activeSessionRef.current = null;
    setSessionId(null);
    setAuthorizeUrl(null);
    setOauthMode(null);
    setUserCode(null);
    setInstructions(null);
    setPollIntervalSeconds(null);
    setDeviceStatus(null);
    setNativeOutput(null);
    setIsDevicePolling(false);
    setDevicePollTick(0);
    setCode("");
    setWaitingForCallback(false);
    clearCallbackPollTimer();
    clearDevicePollTimer();
  }, [clearCallbackPollTimer, clearDevicePollTimer]);

  const isCurrentSession = useCallback(
    (id: string) => activeSessionRef.current?.sessionId === id,
    [],
  );

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    const sessions = activeSessionRef;
    return () => {
      const active = sessions.current;
      if (active?.native) {
        void oauthLogout({
          providerName: active.providerName,
          session_id: active.sessionId,
        });
      }
      resetOAuthState();
      setError(null);
      setCompletionMessage(null);
      setIsLoading(false);
    };
  }, [identity, oauthLogout, resetOAuthState]);

  const handleStartOAuth = async () => {
    if (readonly) return;
    setError(null);
    setCompletionMessage(null);
    setIsLoading(true);
    clearCallbackPollTimer();
    clearDevicePollTimer();
    const startIdentity = identity;
    try {
      const result = await oauthStart({
        providerName,
        mode: loginKind === "console" ? "console" : "max",
      }).unwrap();
      if (startIdentity !== identityRef.current || !mountedRef.current) {
        if (result.mode === "native") {
          void oauthLogout({ providerName, session_id: result.session_id });
        }
        return;
      }
      const mode = inferOAuthMode(baseProvider, result);
      activeSessionRef.current = {
        providerName,
        sessionId: result.session_id,
        native: mode === "native",
      };
      setSessionId(result.session_id);
      setAuthorizeUrl(result.authorize_url);
      setOauthMode(mode);
      setUserCode(result.user_code ?? null);
      setInstructions(result.instructions ?? null);
      setPollIntervalSeconds(
        result.poll_interval ??
          (mode === "native" ? NATIVE_POLL_SECONDS : null),
      );
      setDeviceStatus(null);
      setNativeOutput(null);
      setCode("");
      if (result.authorize_url) openUrl(result.authorize_url);

      if (mode === "callback") {
        setWaitingForCallback(true);
        callbackPollTimerRef.current = setInterval(() => {
          invalidateProvider();
        }, 2000);
      } else {
        setWaitingForCallback(false);
      }

      if (mode === "device" || mode === "native") {
        setDeviceStatus(
          mode === "native"
            ? "Waiting for the sign-in command to finish"
            : "Waiting for device authorization",
        );
        setIsDevicePolling(true);
        setDevicePollTick(0);
      } else {
        setIsDevicePolling(false);
      }
    } catch (e) {
      if (startIdentity === identityRef.current) {
        setError(getErrorMessage(e, "Failed to start OAuth"));
      }
    } finally {
      if (startIdentity === identityRef.current) setIsLoading(false);
    }
  };

  const handlePollDeviceOAuth = useCallback(async () => {
    if (!sessionId) return;
    const polledSession = sessionId;
    const native = oauthMode === "native";
    setError(null);
    setIsLoading(true);
    try {
      const result = await oauthExchange({
        providerName,
        session_id: polledSession,
        code: "",
      }).unwrap();
      if (!isCurrentSession(polledSession)) return;
      if (result.success) {
        resetOAuthState();
        invalidateProviderAndCaps();
        if (native) {
          setCompletionMessage(
            `${serviceName} sign-in finished on the engine host. The engine checks the resulting credentials on the next request; explicit credentials saved for this provider still take precedence.`,
          );
        }
        return;
      }
      if (native) {
        setNativeOutput(result.output ?? null);
        if (result.status !== "pending") {
          activeSessionRef.current = null;
          setIsDevicePolling(false);
          const failure = result.auth_status.trim();
          setError(
            failure && !/^(error|failed)$/i.test(failure)
              ? failure
              : backend === "bedrock"
                ? "AWS SSO sign-in failed on the engine host. Check the AWS profile and SSO configuration, then try again. SSO login is not required when using existing AWS credentials or a Bedrock API key."
                : `${serviceName} sign-in failed on the engine host. Check the cloud account configuration, then try again.`,
          );
          return;
        }
        setDeviceStatus(
          result.auth_status || "Waiting for the sign-in command to finish",
        );
      } else {
        setDeviceStatus(
          result.auth_status || "Waiting for device authorization",
        );
      }
      setPollIntervalSeconds(result.poll_interval ?? pollIntervalSeconds);
      setIsDevicePolling(true);
      setDevicePollTick((tick) => tick + 1);
    } catch (e) {
      if (!isCurrentSession(polledSession)) return;
      setIsDevicePolling(false);
      setError(getErrorMessage(e, "Failed to check authorization"));
    } finally {
      if (isCurrentSession(polledSession) || !activeSessionRef.current) {
        setIsLoading(false);
      }
    }
  }, [
    backend,
    invalidateProviderAndCaps,
    isCurrentSession,
    oauthExchange,
    oauthMode,
    pollIntervalSeconds,
    providerName,
    resetOAuthState,
    serviceName,
    sessionId,
  ]);

  useEffect(() => {
    if (!isDevicePolling || !sessionId) return;
    clearDevicePollTimer();
    const delaySeconds = Math.max(1, pollIntervalSeconds ?? 5);
    devicePollTimerRef.current = setTimeout(() => {
      void handlePollDeviceOAuth();
    }, delaySeconds * 1000);
    return () => {
      clearDevicePollTimer();
    };
  }, [
    clearDevicePollTimer,
    devicePollTick,
    handlePollDeviceOAuth,
    isDevicePolling,
    pollIntervalSeconds,
    sessionId,
  ]);

  useEffect(() => {
    if (waitingForCallback && oauthConnected) {
      resetOAuthState();
      invalidateProviderAndCaps();
    }
  }, [
    invalidateProviderAndCaps,
    oauthConnected,
    resetOAuthState,
    waitingForCallback,
  ]);

  useEffect(() => {
    if (!waitingForCallback) return;
    if (!authStatus) return;
    if (/failed|error|unavailable|missing/i.test(authStatus)) {
      setWaitingForCallback(false);
      clearCallbackPollTimer();
    }
  }, [authStatus, clearCallbackPollTimer, waitingForCallback]);

  const handleExchangeCode = async () => {
    if (!sessionId || !code.trim()) return;
    setError(null);
    setIsLoading(true);
    try {
      const result = await oauthExchange({
        providerName,
        session_id: sessionId,
        code: code.trim(),
      }).unwrap();
      if (!result.success) {
        setError(result.auth_status || "OAuth authorization is not complete");
        return;
      }
      resetOAuthState();
      invalidateProviderAndCaps();
    } catch (e) {
      setError(getErrorMessage(e, "Failed to exchange code"));
    } finally {
      setIsLoading(false);
    }
  };

  const handleLogout = async () => {
    if (readonly) return;
    setError(null);
    setIsLoading(true);
    try {
      await oauthLogout({ providerName }).unwrap();
      resetOAuthState();
      invalidateProviderAndCaps();
    } catch (e) {
      setError(getErrorMessage(e, "Failed to logout"));
    } finally {
      setIsLoading(false);
    }
  };

  const handleCancel = () => {
    resetOAuthState();
  };

  const handleCancelNative = async () => {
    const active = activeSessionRef.current;
    resetOAuthState();
    setIsLoading(false);
    if (!active?.native) return;
    try {
      await oauthLogout({
        providerName: active.providerName,
        session_id: active.sessionId,
      }).unwrap();
    } catch (e) {
      setError(getErrorMessage(e, "Failed to cancel sign-in"));
    }
  };

  const handleOpenAuthorizeUrl = () => {
    if (authorizeUrl) openUrl(authorizeUrl);
  };

  if (oauthConnected) {
    return (
      <Surface className={surfaceClassName} variant="glass" animated="rise">
        <div className={`${styles.headerRow} rf-enter-rise`}>
          <div className={styles.inlineRow}>
            <span className={`${styles.title} ${styles.connected}`}>
              Connected
            </span>
            <span className={styles.copy}>{authStatus}</span>
          </div>
          <Button
            variant="danger"
            size="sm"
            disabled={isLoading || readonly}
            onClick={() => void handleLogout()}
          >
            Disconnect
          </Button>
        </div>
      </Surface>
    );
  }

  if (sessionId && oauthMode === "native") {
    return (
      <Surface className={surfaceClassName} variant="glass" animated="rise">
        <div className={`${styles.title} rf-enter-rise`}>
          Sign in with {serviceName}
        </div>
        <div className={`${styles.copy} rf-enter-rise`}>
          {instructions ??
            "The engine host is running the cloud sign-in command. Follow the instructions below to finish."}
        </div>
        {nativeOutput ? (
          <div className={`${styles.codeBox} rf-enter-rise`}>
            {nativeOutput.split("\n").map((line, index) => (
              <div key={index}>{line}</div>
            ))}
          </div>
        ) : null}
        {isDevicePolling ? (
          <div className={`${styles.copy} rf-enter-rise`} role="status">
            {deviceStatus ?? "Waiting for the sign-in command to finish"}
            {pollIntervalSeconds
              ? ` Checking every ${pollIntervalSeconds} seconds.`
              : ""}
          </div>
        ) : null}
        <div className={`${styles.actionRow} rf-enter-rise`}>
          {!isDevicePolling ? (
            <Button
              variant="primary"
              disabled={isLoading || readonly}
              onClick={() => void handleStartOAuth()}
            >
              Try again
            </Button>
          ) : null}
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void handleCancelNative()}
          >
            Cancel
          </Button>
        </div>
        {error ? (
          <div className={`${styles.errorText} rf-enter-rise`} role="alert">
            {error}
          </div>
        ) : null}
      </Surface>
    );
  }

  if (sessionId && authorizeUrl) {
    if (oauthMode === "device" || userCode) {
      return (
        <Surface className={surfaceClassName} variant="glass" animated="rise">
          <div className={`${styles.title} rf-enter-rise`}>
            Authorize{" "}
            {(PROVIDER_LOGIN_LABELS[baseProvider] ?? "provider").replace(
              "Login with ",
              "",
            )}
          </div>
          <div className={`${styles.copy} rf-enter-rise`}>
            {instructions ??
              "Open the verification page and enter the code shown below."}
          </div>
          {userCode ? (
            <div className="rf-enter-rise">
              <div className={styles.copy}>User code</div>
              <div className={styles.codeBox}>{userCode}</div>
            </div>
          ) : null}
          <div className="rf-enter-rise">
            <div className={styles.copy}>Verification URL</div>
            <a
              href={authorizeUrl}
              className={styles.urlLink}
              onClick={(event) => {
                event.preventDefault();
                handleOpenAuthorizeUrl();
              }}
            >
              {authorizeUrl}
            </a>
          </div>
          <div className={`${styles.copy} rf-enter-rise`}>
            {deviceStatus ?? "Waiting for device authorization"}
            {pollIntervalSeconds
              ? ` Checking every ${pollIntervalSeconds} seconds.`
              : ""}
          </div>
          <div className={`${styles.actionRow} rf-enter-rise`}>
            <Button variant="primary" onClick={handleOpenAuthorizeUrl}>
              Open verification page
            </Button>
            <Button
              variant="soft"
              disabled={isLoading}
              onClick={() => void handlePollDeviceOAuth()}
            >
              {isLoading ? "Checking..." : "Retry"}
            </Button>
            <Button variant="ghost" size="sm" onClick={handleCancel}>
              Cancel
            </Button>
          </div>
          {error ? (
            <div className={`${styles.errorText} rf-enter-rise`}>{error}</div>
          ) : null}
        </Surface>
      );
    }

    if (oauthMode === "callback" && waitingForCallback) {
      return (
        <Surface className={surfaceClassName} variant="glass" animated="rise">
          <div className={`${styles.title} rf-enter-rise`}>
            Waiting for authentication...
          </div>
          <div className={`${styles.copy} rf-enter-rise`}>
            Complete the login in the browser window that opened. This page will
            update automatically.
          </div>
          <div className={`${styles.actionRow} rf-enter-rise`}>
            <span className={styles.copy}>
              Browser didn&apos;t open?{" "}
              <a
                href={authorizeUrl}
                className={styles.urlLink}
                onClick={(event) => {
                  event.preventDefault();
                  handleOpenAuthorizeUrl();
                }}
              >
                Click here
              </a>
            </span>
            <Button variant="ghost" size="sm" onClick={handleCancel}>
              Cancel
            </Button>
          </div>
          {error ? (
            <div className={`${styles.errorText} rf-enter-rise`}>{error}</div>
          ) : null}
        </Surface>
      );
    }

    return (
      <Surface className={surfaceClassName} variant="glass" animated="rise">
        <div className={`${styles.title} rf-enter-rise`}>
          Paste the authorization code
        </div>
        <div className={`${styles.copy} rf-enter-rise`}>
          A browser window should have opened. Log in and copy the code shown on
          the page.
        </div>
        <div className={`${styles.actionRow} rf-enter-rise`}>
          <FieldText
            className={styles.fullWidthInput}
            placeholder="Paste code here..."
            value={code}
            onChange={setCode}
            onKeyDown={(event) => {
              if (event.key === "Enter") void handleExchangeCode();
            }}
          />
          <Button
            variant="primary"
            disabled={isLoading || !code.trim()}
            onClick={() => void handleExchangeCode()}
          >
            {isLoading ? "Connecting..." : "Connect"}
          </Button>
        </div>
        <div className={`${styles.actionRow} rf-enter-rise`}>
          <span className={styles.copy}>
            Browser didn&apos;t open?{" "}
            <a
              href={authorizeUrl}
              className={styles.urlLink}
              onClick={(event) => {
                event.preventDefault();
                handleOpenAuthorizeUrl();
              }}
            >
              Click here
            </a>
          </span>
          <Button variant="ghost" size="sm" onClick={handleCancel}>
            Cancel
          </Button>
        </div>
        {error ? (
          <div className={`${styles.errorText} rf-enter-rise`}>{error}</div>
        ) : null}
      </Surface>
    );
  }

  return (
    <Surface className={surfaceClassName} variant="glass" animated="rise">
      <div className={`${styles.headerRow} rf-enter-rise`}>
        <div className={styles.title}>{loginLabel}</div>
        <Button
          variant="primary"
          disabled={isLoading || readonly}
          onClick={() => void handleStartOAuth()}
        >
          {isLoading ? "Starting..." : "Login"}
        </Button>
      </div>
      {completionMessage ? (
        <div className={`${styles.copy} rf-enter-rise`} role="status">
          {completionMessage}
        </div>
      ) : null}
      {error ? (
        <div className={`${styles.errorText} rf-enter-rise`}>{error}</div>
      ) : null}
    </Surface>
  );
};

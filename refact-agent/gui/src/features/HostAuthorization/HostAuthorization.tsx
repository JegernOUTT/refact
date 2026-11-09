import { useEffect, useId, useRef, useState } from "react";
import { Button, Field, FieldError, FieldText } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { buildApiUrl } from "../../services/refact/apiUrl";
import { appsApi, isAppCatalog } from "../../services/refact/apps";
import { schedulerApi } from "../../services/refact/schedulerApi";
import { selectConfig, setApiKey } from "../Config/configSlice";
import { isAutomationCatalog } from "../Scheduler/automationTypes";
import styles from "./HostAuthorization.module.css";

type Props = {
  resource: "apps" | "automation";
  onAuthorized?: () => void;
};

export function HostAuthorization({ resource, onAuthorized }: Props) {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const [token, setToken] = useState("");
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  const request = useRef<AbortController>();
  const inputId = useId();
  const descriptionId = useId();
  const path = resource === "apps" ? "/v1/apps" : "/v1/scheduler/automation";
  let endpoint: URL | undefined;
  try {
    endpoint = new URL(buildApiUrl(config, path), window.location.href);
  } catch {
    endpoint = undefined;
  }
  const trusted =
    config.host === "web" &&
    config.engineServed === true &&
    config.dev !== true &&
    endpoint !== undefined &&
    endpoint.origin === window.location.origin &&
    ["http:", "https:"].includes(endpoint.protocol) &&
    endpoint.pathname === path &&
    !endpoint.username &&
    !endpoint.password &&
    !endpoint.search &&
    !endpoint.hash;
  const endpointUrl = endpoint?.href;
  useEffect(() => {
    setToken("");
    setFailed(false);
    setPending(false);
    return () => {
      request.current?.abort();
      request.current = undefined;
    };
  }, [trusted, endpointUrl, config.apiKey]);
  const submit = async () => {
    if (!trusted || !endpointUrl || !token.trim() || request.current) return;
    const controller = new AbortController();
    request.current = controller;
    setPending(true);
    setFailed(false);
    const candidate = token.trim();
    setToken("");
    const timeout = window.setTimeout(() => controller.abort(), 15000);
    try {
      const response = await fetch(endpointUrl, {
        method: "GET",
        headers: { Authorization: `Bearer ${candidate}` },
        redirect: "error",
        credentials: "omit",
        cache: "no-store",
        referrerPolicy: "no-referrer",
        signal: controller.signal,
      });
      if (!response.ok || response.redirected || response.url !== endpointUrl)
        throw new Error();
      const data: unknown = await response.json();
      if (
        resource === "apps" ? !isAppCatalog(data) : !isAutomationCatalog(data)
      )
        throw new Error();
      if (controller.signal.aborted) return;
      const accepted = dispatch((innerDispatch, getState) => {
        const current = getState().config;
        if (
          current.host !== "web" ||
          current.engineServed !== true ||
          current.dev === true ||
          current.apiKey !== config.apiKey ||
          new URL(buildApiUrl(current, path), window.location.href).href !==
            endpointUrl
        )
          return false;
        innerDispatch(setApiKey(candidate));
        innerDispatch(appsApi.util.invalidateTags(["Apps", "AppDocument"]));
        innerDispatch(schedulerApi.util.invalidateTags(["Automations"]));
        return true;
      });
      if (accepted) onAuthorized?.();
      else setFailed(true);
    } catch {
      if (request.current === controller) setFailed(true);
    } finally {
      window.clearTimeout(timeout);
      if (request.current === controller) {
        request.current = undefined;
        setPending(false);
      }
    }
  };
  return (
    <section className={styles.authorization} aria-label="Host sign-in">
      <h3>Host sign-in required</h3>
      <p>
        This engine requires its host access token to load Apps and Automation.
        This is not a provider API key or a private source credential.
      </p>
      <p>
        If the worker has no host token configured, its operator must set
        REFACT_DAEMON_TOKEN and restart the engine. There is no default token.
        Never paste the token into chat.
      </p>
      {trusted && endpoint ? (
        <form
          className={styles.form}
          autoComplete="off"
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <p id={descriptionId}>
            Sign in only if you trust this engine at {endpoint.origin}. The
            token is kept in memory for this page session only and is not saved.
          </p>
          {endpoint.protocol === "http:" && (
            <p className={styles.warning}>
              Plain HTTP does not encrypt your token in transit. Use HTTPS or a
              trusted network, especially when connecting to a remote host.
            </p>
          )}
          <Field label="Host access token" htmlFor={inputId}>
            <FieldText
              id={inputId}
              type="password"
              autoComplete="off"
              spellCheck={false}
              autoCapitalize="none"
              aria-describedby={descriptionId}
              value={token}
              disabled={pending}
              onChange={setToken}
            />
          </Field>
          {failed && (
            <FieldError>
              Sign-in could not be verified. Check the host token and engine
              configuration, then try again.
            </FieldError>
          )}
          <Button
            type="submit"
            disabled={pending || !token.trim()}
            loading={pending}
          >
            Sign in to host
          </Button>
        </form>
      ) : (
        <p role="status">
          {config.host !== "web"
            ? "Your IDE supplies the engine token. Reconnect the IDE backend or ask its operator to check the host token configuration."
            : "For safe token entry, open the GUI served directly by the trusted engine. Token entry is disabled for development proxies and other backend destinations."}
        </p>
      )}
    </section>
  );
}

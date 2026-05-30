import { useEffect } from "react";
import { useLocalStorage } from "usehooks-ts";
import { isOpenExternalUrl } from "../events/setup";
import { useAppDispatch } from "./useAppDispatch";
import { useConfig } from "./useConfig";
import { updateConfig } from "../features/Config/configSlice";
import { sanitizeEngineBaseUrl } from "../services/refact/apiUrl";

function currentWindowPort(): number {
  return (
    Number(window.location.port) ||
    (window.location.protocol === "https:" ? 443 : 80)
  );
}

export function resolveWebLspUrl(
  config: {
    dev?: boolean;
    engineServed?: boolean;
    lspUrl?: string;
  },
  storedLspUrl: string,
): string {
  if (config.engineServed) return "";

  const configLspUrl = sanitizeEngineBaseUrl(config.lspUrl) ?? "";
  if (config.dev) return configLspUrl;

  return sanitizeEngineBaseUrl(storedLspUrl) ?? configLspUrl;
}

// all of the events that are normally handeled by the IDE
// are handled here for the web version.
export function useEventBusForWeb() {
  const config = useConfig();
  const [lspUrl] = useLocalStorage("lspUrl", "");
  const [apiKey] = useLocalStorage("apiKey", "");
  const dispatch = useAppDispatch();

  useEffect(() => {
    if (config.host !== "web") {
      return;
    }

    const listener = (event: MessageEvent) => {
      if (event.source !== window) {
        return;
      }

      if (isOpenExternalUrl(event.data)) {
        const { url } = event.data.payload;
        window.open(url, "_blank")?.focus();
      }
    };

    window.addEventListener("message", listener);

    return () => {
      window.removeEventListener("message", listener);
    };
  }, [config.host]);

  useEffect(() => {
    if (config.host !== "web") {
      return;
    }

    dispatch(
      updateConfig({
        lspUrl: resolveWebLspUrl(config, lspUrl),
        lspPort: config.engineServed ? currentWindowPort() : config.lspPort,
        apiKey,
      }),
    );
  }, [apiKey, lspUrl, dispatch, config]);
}

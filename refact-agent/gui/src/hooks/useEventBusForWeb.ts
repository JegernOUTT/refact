import { useEffect } from "react";
import { useLocalStorage } from "usehooks-ts";
import { isOpenExternalUrl } from "../events/setup";
import { useAppDispatch } from "./useAppDispatch";
import { useConfig } from "./useConfig";
import { updateConfig } from "../features/Config/configSlice";

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

    const origin = window.location.origin;
    const nextLspUrl = config.engineServed ? origin : lspUrl || config.lspUrl;
    const nextPort = config.engineServed
      ? Number(window.location.port) ||
        (window.location.protocol === "https:" ? 443 : 80)
      : config.lspPort;

    dispatch(
      updateConfig({
        lspUrl: nextLspUrl,
        lspPort: nextPort,
        apiKey,
      }),
    );
  }, [
    apiKey,
    lspUrl,
    dispatch,
    config.host,
    config.lspUrl,
    config.lspPort,
    config.engineServed,
  ]);
}

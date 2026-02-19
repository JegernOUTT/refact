import { useEffect, useRef } from "react";
import { useLocalStorage } from "usehooks-ts";
import { isLogOut, isOpenExternalUrl, isSetupHost } from "../events/setup";
import { useAppDispatch } from "./useAppDispatch";
import { useConfig } from "./useConfig";
import { updateConfig } from "../features/Config/configSlice";

// all of the events that are normally handeled by the IDE
// are handled here for the web version.
export function useEventBusForWeb() {
  const config = useConfig();
  const [addressURL, setAddressURL] = useLocalStorage("lspUrl", "");
  const [apiKey, setApiKey] = useLocalStorage("apiKey", "");
  const dispatch = useAppDispatch();
  const addressURLRef = useRef(addressURL);
  const apiKeyRef = useRef(apiKey);
  addressURLRef.current = addressURL;
  apiKeyRef.current = apiKey;

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

      if (isSetupHost(event.data)) {
        const { host } = event.data.payload;
        setAddressURL("Refact");
        setApiKey(host.apiKey);
        dispatch(
          updateConfig({
            addressURL: addressURLRef.current,
            apiKey: apiKeyRef.current,
          }),
        );
      }

      if (isLogOut(event.data)) {
        setAddressURL("");
        setApiKey("");
        dispatch(
          updateConfig({
            addressURL: addressURLRef.current,
            apiKey: apiKeyRef.current,
          }),
        );
      }
    };

    window.addEventListener("message", listener);

    return () => {
      window.removeEventListener("message", listener);
    };
  }, [setApiKey, setAddressURL, config.host, dispatch]);

  useEffect(() => {
    if (config.host !== "web") {
      return;
    }
    dispatch(updateConfig({ addressURL, apiKey }));
  }, [apiKey, addressURL, dispatch, config.host]);
}

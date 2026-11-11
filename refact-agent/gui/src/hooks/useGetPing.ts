import { useEffect } from "react";

import { selectConfig } from "../features/Config/configSlice";
import { pingApi } from "../services/refact/ping";
import { useAppSelector } from "./useAppSelector";
import { useAppDispatch } from "./useAppDispatch";
import { setBackendStatus } from "../features/Connection";
import { hasUsableEngineEndpoint } from "../services/refact/apiUrl";
import { useIsDocumentVisible } from "./useIsDocumentVisible";

export const useGetPing = () => {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const args = {
    host: config.host,
    dev: config.dev,
    engineServed: config.engineServed,
    lspPort: config.lspPort,
    lspUrl: config.lspUrl,
  };
  const canPing = hasUsableEngineEndpoint(config);
  const visible = useIsDocumentVisible(() => {
    if (canPing)
      void dispatch(
        pingApi.endpoints.ping.initiate(args, {
          subscribe: false,
          forceRefetch: true,
        }),
      );
  });
  const cached = pingApi.endpoints.ping.useQueryState(args);
  const interval = cached.isError ? 2000 : 8000;
  const result = pingApi.endpoints.ping.useQuery(args, {
    pollingInterval: visible ? interval : 0,
    refetchOnMountOrArgChange: false,
    skip: !canPing,
  });

  useEffect(() => {
    if (canPing) return;
    dispatch(
      setBackendStatus({
        status: "unknown",
        error: "Backend port is not available",
      }),
    );
  }, [canPing, dispatch]);

  return result;
};

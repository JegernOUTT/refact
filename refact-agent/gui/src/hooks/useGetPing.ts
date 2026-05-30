import { useState, useEffect } from "react";

import { selectConfig } from "../features/Config/configSlice";
import { pingApi } from "../services/refact";
import { useAppSelector } from "./useAppSelector";
import { useAppDispatch } from "./useAppDispatch";
import { setBackendStatus } from "../features/Connection";
import { hasUsableEngineEndpoint } from "../services/refact/apiUrl";

const POLL_INTERVAL_HEALTHY = 5000;
const POLL_INTERVAL_ERROR = 2000;

export const useGetPing = () => {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const currentHost = config.host;
  const currentDev = config.dev;
  const currentEngineServed = config.engineServed;
  const currentLspPort = config.lspPort;
  const currentLspUrl = config.lspUrl;
  const canPing = hasUsableEngineEndpoint(config);

  const [pollingInterval, setPollingInterval] = useState(POLL_INTERVAL_ERROR);
  const [queryStarted, setQueryStarted] = useState(false);

  const result = pingApi.endpoints.ping.useQuery(
    {
      host: currentHost,
      dev: currentDev,
      engineServed: currentEngineServed,
      lspPort: currentLspPort,
      lspUrl: currentLspUrl,
    },
    {
      pollingInterval,
      refetchOnMountOrArgChange: true,
      skip: !canPing,
    },
  );

  useEffect(() => {
    if (canPing) return;
    setPollingInterval(POLL_INTERVAL_ERROR);
    setQueryStarted(false);
    dispatch(
      setBackendStatus({
        status: "unknown",
        error: "Backend port is not available",
      }),
    );
  }, [canPing, dispatch]);

  useEffect(() => {
    if (result.requestId && !queryStarted) {
      setQueryStarted(true);
    }
  }, [result.requestId, queryStarted]);

  useEffect(() => {
    if (result.isUninitialized && queryStarted) {
      setPollingInterval(POLL_INTERVAL_ERROR);
      setQueryStarted(false);
    } else if (result.isSuccess) {
      setPollingInterval(POLL_INTERVAL_HEALTHY);
      dispatch(setBackendStatus({ status: "online" }));
    } else if (result.isError) {
      setPollingInterval(POLL_INTERVAL_ERROR);
      const err = result.error as Record<string, unknown> | undefined;
      const errorMsg =
        err && typeof err === "object" && "message" in err
          ? String(err.message)
          : "Connection failed";
      dispatch(setBackendStatus({ status: "offline", error: errorMsg }));
    }
  }, [
    result.isSuccess,
    result.isError,
    result.isUninitialized,
    result.error,
    queryStarted,
    canPing,
    dispatch,
  ]);

  useEffect(() => {
    setPollingInterval(POLL_INTERVAL_ERROR);
    setQueryStarted(false);
  }, [
    currentHost,
    currentDev,
    currentEngineServed,
    currentLspPort,
    currentLspUrl,
  ]);

  return result;
};

import { useEffect } from "react";
import { QueryStatus } from "@reduxjs/toolkit/query";
import { useAppSelector } from "./useAppSelector";
import { useAppDispatch } from "./useAppDispatch";
import { selectConfig } from "../features/Config/configSlice";
import { getIsAuthError } from "../features/Errors/errorsSlice";
import { selectBackendStatus } from "../features/Connection";
import { capsApi } from "../services/refact/caps";
import { useGetPing } from "./useGetPing";
import { hasUsableEngineEndpoint } from "../services/refact/apiUrl";

const EMPTY_CAPS_POLLING_INTERVAL_MS = 2000;
const STARTUP_CAPS_POLLING_INTERVAL_MS = 5000;

export const useGetCapsQuery = (_args?: undefined) => {
  const dispatch = useAppDispatch();
  const isAuthError = useAppSelector(getIsAuthError);
  const config = useAppSelector(selectConfig);
  const backendStatus = useAppSelector(selectBackendStatus);
  const cachedCaps = useAppSelector(
    (state) => capsApi.endpoints.getCaps.select(undefined)(state).data,
  );
  const cachedCapsStatus = useAppSelector(
    (state) => capsApi.endpoints.getCaps.select(undefined)(state).status,
  );
  useGetPing();
  const canFetchCaps = hasUsableEngineEndpoint(config);
  const skip = !!isAuthError || !canFetchCaps || backendStatus === "offline";
  const isCapsUninitialized = cachedCapsStatus === QueryStatus.uninitialized;
  const hasLoadedEmptyModelList =
    !!cachedCaps && Object.keys(cachedCaps.chat_models).length === 0;
  const pollingInterval = hasLoadedEmptyModelList
    ? EMPTY_CAPS_POLLING_INTERVAL_MS
    : isCapsUninitialized
      ? STARTUP_CAPS_POLLING_INTERVAL_MS
      : 0;
  const caps = capsApi.useGetCapsQuery(undefined, {
    pollingInterval,
    skip,
    skipPollingIfUnfocused: true,
  });

  useEffect(() => {
    if (skip) return;
    const refetch = () => {
      void dispatch(
        capsApi.endpoints.getCaps.initiate(undefined, {
          subscribe: false,
          forceRefetch: true,
        }),
      );
    };
    window.addEventListener("focus", refetch);
    window.addEventListener("online", refetch);
    return () => {
      window.removeEventListener("focus", refetch);
      window.removeEventListener("online", refetch);
    };
  }, [dispatch, skip]);

  return caps;
};

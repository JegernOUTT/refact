import { QueryStatus } from "@reduxjs/toolkit/query";
import { useAppSelector } from "./useAppSelector";
import { selectConfig } from "../features/Config/configSlice";
import { getIsAuthError } from "../features/Errors/errorsSlice";
import { capsApi } from "../services/refact/caps";
import { useGetPing } from "./useGetPing";

const EMPTY_CAPS_POLLING_INTERVAL_MS = 2000;
const STARTUP_CAPS_POLLING_INTERVAL_MS = 5000;

export const useGetCapsQuery = (_args?: undefined) => {
  const isAuthError = useAppSelector(getIsAuthError);
  const currentLspPort = useAppSelector(selectConfig).lspPort;
  const cachedCaps = useAppSelector(
    (state) => capsApi.endpoints.getCaps.select(undefined)(state).data,
  );
  const cachedCapsStatus = useAppSelector(
    (state) => capsApi.endpoints.getCaps.select(undefined)(state).status,
  );
  useGetPing();
  const canFetchCaps = Number.isFinite(currentLspPort) && currentLspPort > 0;
  const skip = !!isAuthError || !canFetchCaps;
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
    refetchOnFocus: true,
    refetchOnReconnect: true,
    skip,
    skipPollingIfUnfocused: true,
  });

  return caps;
};

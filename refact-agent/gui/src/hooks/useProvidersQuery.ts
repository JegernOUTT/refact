import { useCallback, useMemo, useState } from "react";
import type { RootState } from "../app/store";
import { capsApi, providersApi } from "../services/refact";
import { useAppSelector } from "./useAppSelector";
import { selectBackendStatus } from "../features/Connection";
import { selectConfig } from "../features/Config/configSlice";
import { hasAnyUsableActiveProvider } from "../features/Login/providerAccess";
import { useGetCapsQuery } from "./useGetCapsQuery";
import {
  hasReadyPluginBackend,
  hasUsableEngineEndpoint,
} from "../services/refact/apiUrl";

export type ProviderBootstrapStatus =
  | "backend_connecting"
  | "backend_installing"
  | "backend_offline"
  | "provider_loading"
  | "provider_error"
  | "setup_required"
  | "ready";

export function useGetConfiguredProvidersQuery() {
  const backendStatus = useAppSelector(selectBackendStatus);
  const config = useAppSelector(selectConfig);
  return providersApi.useGetConfiguredProvidersQuery(undefined, {
    skip: backendStatus === "offline" || !hasUsableEngineEndpoint(config),
    refetchOnMountOrArgChange: true,
    refetchOnFocus: true,
    refetchOnReconnect: true,
  });
}

function refetchIfStarted(refetch: () => Promise<unknown>): Promise<unknown> {
  try {
    return refetch();
  } catch {
    return Promise.resolve();
  }
}

function selectCapsQueryIsReady(state: RootState) {
  const queryState = capsApi.endpoints.getCaps.select(undefined)(state);
  return queryState.isSuccess || queryState.data !== undefined;
}

export function useProviderBootstrapState() {
  const backendStatus = useAppSelector(selectBackendStatus);
  const config = useAppSelector(selectConfig);
  const providersQuery = useGetConfiguredProvidersQuery();
  const capsQuery = useGetCapsQuery();
  const capsQueryIsReady = useAppSelector(selectCapsQueryIsReady);
  const hasAnyActiveProvider = useMemo(() => {
    return hasAnyUsableActiveProvider({
      providers: providersQuery.data?.providers ?? [],
    });
  }, [providersQuery.data?.providers]);

  const providersLoading = !providersQuery.isSuccess && !providersQuery.isError;
  const capsLoading = !capsQueryIsReady && !capsQuery.isError;

  const [isRetrying, setIsRetrying] = useState(false);
  const refetchProviders = providersQuery.refetch;
  const refetchCaps = capsQuery.refetch;
  const retry = useCallback(() => {
    setIsRetrying(true);
    void Promise.allSettled([
      refetchIfStarted(refetchProviders),
      refetchIfStarted(refetchCaps),
    ]).finally(() => {
      setIsRetrying(false);
    });
  }, [refetchProviders, refetchCaps]);

  let status: ProviderBootstrapStatus = "provider_loading";
  if (!hasReadyPluginBackend(config)) {
    if (config.connectionStatus === "failed") {
      status = "backend_offline";
    } else if (config.connectionStatus === "installing") {
      status = "backend_installing";
    } else {
      status = "backend_connecting";
    }
  } else if (backendStatus === "unknown") {
    status = "backend_connecting";
  } else if (backendStatus === "offline") {
    status = "backend_offline";
  } else if (providersLoading || capsLoading) {
    status = "provider_loading";
  } else if (providersQuery.isError || capsQuery.isError) {
    status = "provider_error";
  } else if (hasAnyActiveProvider) {
    status = "ready";
  } else {
    status = "setup_required";
  }

  return {
    backendStatus,
    providersQuery,
    capsQuery,
    status,
    hasAnyActiveProvider,
    canAccessApp: status === "ready",
    canShowProviderSetup: status === "setup_required",
    retry,
    isRetrying,
  };
}

export function useGetProviderQuery({
  providerName,
}: {
  providerName: string;
}) {
  return providersApi.useGetProviderQuery({ providerName });
}

export function useGetProviderSchemaQuery({
  providerName,
}: {
  providerName: string;
}) {
  return providersApi.useGetProviderSchemaQuery({ providerName });
}

export function useGetProviderModelsQuery({
  providerName,
}: {
  providerName: string;
}) {
  return providersApi.useGetProviderModelsQuery({ providerName });
}

export function useUpdateProviderMutation() {
  return providersApi.useUpdateProviderMutation();
}

export function useDeleteProviderMutation() {
  return providersApi.useDeleteProviderMutation();
}

export function useGetDefaultsQuery() {
  return providersApi.useGetDefaultsQuery(undefined);
}

export function useUpdateDefaultsMutation() {
  return providersApi.useUpdateDefaultsMutation();
}

import { providersApi } from "../services/refact";
import { useAppSelector } from "./useAppSelector";
import { selectBackendStatus } from "../features/Connection";

export function useGetConfiguredProvidersQuery() {
  const backendStatus = useAppSelector(selectBackendStatus);
  return providersApi.useGetConfiguredProvidersQuery(undefined, {
    skip: backendStatus !== "online",
    refetchOnMountOrArgChange: true,
    refetchOnFocus: true,
    refetchOnReconnect: true,
  });
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

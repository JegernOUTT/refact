import { useMemo } from "react";
import type { ProviderListItem } from "../../../services/refact";
import { getProviderName } from "../getProviderName";

function getPriority(provider: ProviderListItem) {
  if (provider.base_provider === "custom") return 2;
  return 1;
}

export function useGetConfiguredProvidersView({
  configuredProviders,
}: {
  configuredProviders: ProviderListItem[];
}) {
  const sortedConfiguredProviders = useMemo(() => {
    return [...configuredProviders].sort((a, b) => {
      const priorityA = getPriority(a);
      const priorityB = getPriority(b);

      if (priorityA !== priorityB) {
        return priorityA - priorityB;
      }

      const baseProviderCompare = a.base_provider.localeCompare(
        b.base_provider,
      );
      if (baseProviderCompare !== 0) return baseProviderCompare;

      const displayNameCompare = getProviderName(a).localeCompare(
        getProviderName(b),
      );
      if (displayNameCompare !== 0) return displayNameCompare;

      return a.name.localeCompare(b.name);
    });
  }, [configuredProviders]);

  return {
    sortedConfiguredProviders,
  };
}

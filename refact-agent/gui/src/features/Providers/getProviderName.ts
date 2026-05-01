import { BEAUTIFUL_PROVIDER_NAMES } from "./constants";

export type ProviderNameInput = {
  name: string;
  base_provider?: string;
  display_name?: string;
};

function getBeautifulProviderName(providerName: string): string | undefined {
  return BEAUTIFUL_PROVIDER_NAMES[providerName] as string | undefined;
}

export function getProviderName(provider: ProviderNameInput | string): string {
  if (typeof provider === "string") {
    return getBeautifulProviderName(provider) ?? provider;
  }

  const displayName = provider.display_name?.trim();
  if (displayName) return displayName;

  const baseProvider = provider.base_provider?.trim();
  if (baseProvider) {
    const beautyName = getBeautifulProviderName(baseProvider);
    if (beautyName) return beautyName;
  }

  const maybeName = provider.name.trim();
  if (!maybeName) return "Unknown Provider";
  return getBeautifulProviderName(maybeName) ?? maybeName;
}

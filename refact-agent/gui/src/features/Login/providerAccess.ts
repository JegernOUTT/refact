import type { ProviderListItem } from "../../services/refact";

export function hasAnyUsableActiveProvider({
  providers,
  addressURL,
  apiKey,
}: {
  providers: ProviderListItem[];
  addressURL?: string;
  apiKey?: string | null;
}): boolean {
  return providers.some((provider) => {
    if (provider.status !== "active") return false;

    if (provider.name !== "refact") {
      return true;
    }

    const normalizedAddress = (addressURL ?? "").trim().toLowerCase();
    const normalizedApiKey = (apiKey ?? "").trim();

    return normalizedAddress === "refact" && normalizedApiKey.length > 0;
  });
}

import type { ProviderListItem } from "../../services/refact/providers";

export function resolveSelectedProvider(
  currentModel: string | undefined,
  providers: ProviderListItem[] | undefined,
): ProviderListItem | null {
  if (!currentModel || !providers) return null;
  const slashIndex = currentModel.indexOf("/");
  if (slashIndex <= 0) return null;
  const providerName = currentModel.slice(0, slashIndex);
  return providers.find((provider) => provider.name === providerName) ?? null;
}

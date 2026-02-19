import { BEAUTIFUL_PROVIDER_NAMES } from "./constants";

export function getProviderName(provider: { name: string } | string): string {
  if (typeof provider === "string") return BEAUTIFUL_PROVIDER_NAMES[provider];
  const maybeName = provider.name;
  if (!maybeName) return "Unknown Provider";
  const beautyName = BEAUTIFUL_PROVIDER_NAMES[maybeName] as string | undefined;
  return beautyName ? beautyName : maybeName;
}

import type { ProviderListItem } from "../../../services/refact";
import { BEAUTIFUL_PROVIDER_NAMES, HIDDEN_PROVIDER_BASES } from "../constants";

export type ProviderBaseOption = {
  id: string;
  label: string;
};

const INSTANCE_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_-]*$/;
const PROVIDER_ID_PREFIX = /^[A-Za-z0-9]/;
const RESERVED_INSTANCE_IDS = new Set(["defaults", "refact"]);
const MAX_INSTANCE_ID_LENGTH = 64;

function titleCaseProviderId(providerId: string) {
  return providerId
    .split("_")
    .filter((part) => part.length > 0)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

export function providerBaseLabel(baseProvider: string) {
  const beautifulName = BEAUTIFUL_PROVIDER_NAMES[baseProvider] as
    | string
    | undefined;
  return beautifulName ?? titleCaseProviderId(baseProvider);
}

export function providerInstanceDisplayName(
  baseProvider: string,
  instanceId: string,
) {
  const suffix = instanceId.startsWith(`${baseProvider}_`)
    ? instanceId.slice(baseProvider.length + 1)
    : "";
  return suffix
    ? `${providerBaseLabel(baseProvider)} ${suffix}`
    : providerBaseLabel(baseProvider);
}

export function nextInstanceId(baseProvider: string, providerNames: string[]) {
  const existingNames = new Set(providerNames);
  for (let index = 2; ; index += 1) {
    const candidate = `${baseProvider}_${index}`;
    if (!existingNames.has(candidate)) return candidate;
  }
}

export function providerBaseOptions(
  providers: ProviderListItem[],
): ProviderBaseOption[] {
  const hiddenBaseIds = new Set<string>(HIDDEN_PROVIDER_BASES);
  const baseIds = new Set<string>();

  for (const provider of providers) {
    const baseProvider = provider.base_provider.trim();
    if (!baseProvider || hiddenBaseIds.has(baseProvider)) continue;
    if (baseProvider === "anthropic") {
      baseIds.add("claude_code");
      continue;
    }
    baseIds.add(baseProvider);
  }

  return [...baseIds]
    .sort((a, b) => providerBaseLabel(a).localeCompare(providerBaseLabel(b)))
    .map((id) => ({ id, label: providerBaseLabel(id) }));
}

export function validateProviderInstanceId(
  instanceId: string,
  providerNames: string[],
) {
  const trimmedInstanceId = instanceId.trim();
  if (!trimmedInstanceId) return "Instance id is required.";
  if (trimmedInstanceId.length > MAX_INSTANCE_ID_LENGTH) {
    return "Instance id must be 64 characters or fewer.";
  }
  if (RESERVED_INSTANCE_IDS.has(trimmedInstanceId.toLowerCase())) {
    return "This instance id is reserved.";
  }
  if (!PROVIDER_ID_PREFIX.test(trimmedInstanceId)) {
    return "Instance id must start with an ASCII letter or digit.";
  }
  if (
    trimmedInstanceId.includes(".") ||
    trimmedInstanceId.includes("/") ||
    trimmedInstanceId.includes("\\")
  ) {
    return "Instance id must not contain path characters.";
  }
  if (!INSTANCE_ID_PATTERN.test(trimmedInstanceId)) {
    return "Use ASCII letters, numbers, underscores, and hyphens only.";
  }
  if (
    providerNames.some(
      (providerName) =>
        providerName.toLowerCase() === trimmedInstanceId.toLowerCase(),
    )
  ) {
    return "A provider with this id already exists.";
  }
  return null;
}

import type { ProviderFormValues } from "./useProviderForm";

export type AggregatedProviderFields = {
  importantFields: Record<string, string | boolean>;
  extraFields: Record<string, string | boolean>;
};

const EXTRA_FIELDS_KEYS = [
  "embedding_endpoint",
  "completion_endpoint",
  "chat_endpoint",
  "tokenizer_api_key",
];

const HIDDEN_FIELDS_KEYS = [
  "name",
  "readonly",
  "enabled",
  "supports_completion",
];

export function aggregateProviderFields(providerData: ProviderFormValues) {
  return Object.entries(providerData).reduce<AggregatedProviderFields>(
    (acc, [key, value]) => {
      if (HIDDEN_FIELDS_KEYS.some((hiddenField) => hiddenField === key)) {
        return acc;
      }

      if (typeof value === "object" && value !== null) {
        return acc;
      }

      const fieldValue = value as string | boolean;

      if (EXTRA_FIELDS_KEYS.some((extraField) => extraField === key)) {
        acc.extraFields[key] = fieldValue;
      } else {
        acc.importantFields[key] = fieldValue;
      }

      return acc;
    },
    { importantFields: {}, extraFields: {} },
  );
}

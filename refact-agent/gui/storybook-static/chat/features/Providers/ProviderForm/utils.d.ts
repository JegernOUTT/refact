import type { ProviderFormValues } from "./useProviderForm";
export type AggregatedProviderFields = {
    importantFields: Record<string, string | boolean>;
    extraFields: Record<string, string | boolean>;
};
export declare function aggregateProviderFields(providerData: ProviderFormValues): AggregatedProviderFields;

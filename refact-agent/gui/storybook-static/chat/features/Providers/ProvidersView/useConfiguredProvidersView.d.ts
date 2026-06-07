import type { ProviderListItem } from "../../../services/refact";
export declare function useGetConfiguredProvidersView({ configuredProviders, }: {
    configuredProviders: ProviderListItem[];
}): {
    sortedConfiguredProviders: ProviderListItem[];
};

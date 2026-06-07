import type { ProviderCardProps } from "./ProviderCard";
export declare const useUpdateProvider: ({ provider, }: {
    provider: ProviderCardProps["provider"];
}) => {
    updateProviderEnabledState: () => Promise<void>;
    isUpdatingEnabledState: boolean;
};

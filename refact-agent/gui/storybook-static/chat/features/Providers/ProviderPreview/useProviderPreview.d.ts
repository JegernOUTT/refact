import { MutationActionCreatorResult, MutationDefinition, BaseQueryFn, FetchArgs, FetchBaseQueryError, FetchBaseQueryMeta } from '@reduxjs/toolkit/query';
import { ProviderListItem } from "../../../services/refact";
import type { ProviderFormValues } from "../ProviderForm/useProviderForm";
export declare function useProviderPreview(handleSetCurrentProvider: (provider: ProviderListItem | null) => void): {
    updateProvider: (arg: {
        providerName: string;
        settings: Record<string, unknown>;
    }) => MutationActionCreatorResult<MutationDefinition<{
        providerName: string;
        settings: Record<string, unknown>;
    }, BaseQueryFn<string | FetchArgs, unknown, FetchBaseQueryError, {}, FetchBaseQueryMeta>, "PROVIDERS" | "PROVIDER" | "PROVIDER_SCHEMA" | "PROVIDER_MODELS" | "AVAILABLE_MODELS" | "DEFAULTS", {
        success: boolean;
    }, "providers">>;
    handleDeleteProvider: (providerName: string) => Promise<void>;
    handleDiscardChanges: () => void;
    handleSaveChanges: (updatedProviderData: ProviderFormValues, providerName: string) => Promise<void>;
    isSavingProvider: boolean;
    isDeletingProvider: boolean;
    currentProviderName: string;
};

import { Dispatch, SetStateAction } from 'react';
import { ConfiguredProvidersResponse, ProviderDetailResponse } from '../../../services/refact';
import type { SchemaFieldDef } from "./SchemaField";
export type ProviderFormValues = {
    enabled: boolean;
    readonly: boolean;
    base_provider: string;
    display_name: string;
    [key: string]: unknown;
};
type ParsedSchema = {
    fields: SchemaFieldDef[];
    oauth?: {
        supported: boolean;
        methods?: {
            id: string;
            label: string;
            description?: string;
        }[];
    };
    description?: string;
};
export declare function useProviderForm({ providerName }: {
    providerName: string;
}): {
    formValues: ProviderFormValues | null;
    parsedSchema: ParsedSchema | null;
    importantFields: SchemaFieldDef[];
    extraFields: SchemaFieldDef[];
    areShowingExtraFields: boolean;
    setAreShowingExtraFields: Dispatch<SetStateAction<boolean>>;
    handleFieldSave: (key: string, value: unknown) => Promise<void>;
    configuredProviders: ConfiguredProvidersResponse | undefined;
    detailedProvider: ProviderDetailResponse | undefined;
    isProviderLoadedSuccessfully: boolean;
};
export {};

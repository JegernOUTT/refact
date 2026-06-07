export type ProviderNameInput = {
    name: string;
    base_provider?: string;
    display_name?: string;
};
export declare function getProviderName(provider: ProviderNameInput | string): string;

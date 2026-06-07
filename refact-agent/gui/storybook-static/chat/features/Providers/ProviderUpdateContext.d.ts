import React, { ReactNode } from "react";
type ProviderUpdateState = {
    updatingProviders: Record<string, boolean>;
    setProviderUpdating: (providerName: string, isUpdating: boolean) => void;
};
export declare const ProviderUpdateProvider: React.FC<{
    children: ReactNode;
}>;
export declare const useProviderUpdateContext: () => ProviderUpdateState;
export {};

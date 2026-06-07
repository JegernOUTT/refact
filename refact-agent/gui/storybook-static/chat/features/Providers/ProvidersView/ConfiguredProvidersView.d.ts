import React from "react";
import type { ProviderListItem } from "../../../services/refact";
export type ConfiguredProvidersViewProps = {
    configuredProviders: ProviderListItem[];
    handleSetCurrentProvider: (provider: ProviderListItem) => void;
    onAddInstance: () => void;
    onDuplicateProvider: (provider: ProviderListItem) => void;
};
export declare const ConfiguredProvidersView: React.FC<ConfiguredProvidersViewProps>;

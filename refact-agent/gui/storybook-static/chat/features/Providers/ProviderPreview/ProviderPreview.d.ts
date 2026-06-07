import React from "react";
import type { ProviderListItem } from "../../../services/refact";
export type ProviderPreviewProps = {
    configuredProviders: ProviderListItem[];
    currentProvider: ProviderListItem;
    handleSetCurrentProvider: (provider: ProviderListItem | null) => void;
    onDuplicateProvider?: (provider: ProviderListItem) => void;
};
export declare const ProviderPreview: React.FC<ProviderPreviewProps>;

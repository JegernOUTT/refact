import React from "react";
import type { ProviderListItem } from "../../../services/refact";
export type ProviderCardProps = {
    provider: ProviderListItem;
    setCurrentProvider: (provider: ProviderListItem) => void;
    onDuplicateProvider?: (provider: ProviderListItem) => void;
};
export declare const ProviderCard: React.FC<ProviderCardProps>;

import React from "react";
import type { ExtensionMarketplaceSource } from "../../services/refact/extensionsMarketplace";
type MarketplaceSourceSelectorProps = {
    sources: ExtensionMarketplaceSource[];
    selectedSource: string | null;
    onSelectSource: (sourceId: string | null) => void;
    onOpenSettings: () => void;
};
export declare const MarketplaceSourceSelector: React.FC<MarketplaceSourceSelectorProps>;
export {};

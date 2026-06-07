import React from "react";
import type { MarketplaceSource } from "../../services/refact/mcpMarketplace";
type SourceSelectorProps = {
    sources: MarketplaceSource[];
    selectedSource: string | null;
    onSelectSource: (sourceId: string | null) => void;
    onOpenSettings: () => void;
};
export declare const SourceSelector: React.FC<SourceSelectorProps>;
export {};

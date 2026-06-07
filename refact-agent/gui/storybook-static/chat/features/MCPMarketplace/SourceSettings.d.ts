import React from "react";
import type { MarketplaceSource } from "../../services/refact/mcpMarketplace";
type SourceSettingsProps = {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    sources: MarketplaceSource[];
};
export declare const SourceSettings: React.FC<SourceSettingsProps>;
export {};

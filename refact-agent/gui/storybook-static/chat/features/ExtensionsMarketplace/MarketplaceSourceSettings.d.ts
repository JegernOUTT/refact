import React from "react";
import type { ExtensionMarketplaceSource } from "../../services/refact/extensionsMarketplace";
type MarketplaceSourceSettingsProps = {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    sources: ExtensionMarketplaceSource[];
};
export declare const MarketplaceSourceSettings: React.FC<MarketplaceSourceSettingsProps>;
export {};

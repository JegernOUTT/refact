import React from "react";
import type { ExtensionMarketplaceItem } from "../../services/refact/extensionsMarketplace";
type MarketplaceItemCardProps = {
    item: ExtensionMarketplaceItem;
    isInstalling: boolean;
    onInstall: (item: ExtensionMarketplaceItem) => void;
};
export declare const MarketplaceItemCard: React.FC<MarketplaceItemCardProps>;
export {};

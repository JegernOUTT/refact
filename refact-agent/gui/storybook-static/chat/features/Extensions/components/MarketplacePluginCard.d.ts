import React from "react";
import type { PluginEntry } from "../../../services/refact/plugins";
export type MarketplacePluginCardProps = {
    plugin: PluginEntry;
    isInstalled: boolean;
};
export declare const MarketplacePluginCard: React.FC<MarketplacePluginCardProps>;

import React from "react";
import type { ExtensionMarketplaceItem } from "../../services/refact/extensionsMarketplace";
type MarketplaceInstallDialogProps = {
    open: boolean;
    item: ExtensionMarketplaceItem | null;
    hasProjectRoot: boolean;
    isInstalling: boolean;
    isConflict: boolean;
    error: string | null;
    onOpenChange: (open: boolean) => void;
    onInstall: (scope: "local" | "global", params: Record<string, string>, overwrite: boolean) => void;
};
export declare const MarketplaceInstallDialog: React.FC<MarketplaceInstallDialogProps>;
export {};

import React from "react";
import type { ModelCapabilities } from "../../features/Providers/ProviderForm/ProviderModelsList/utils/groupModelsWithPricing";
import type { CapCost } from "../../services/refact";
export type RichModelData = {
    displayName: string;
    pricing?: CapCost;
    nCtx?: number;
    capabilities?: ModelCapabilities;
    isDefault?: boolean;
    isChat2?: boolean;
    isTaskPlannerAgent?: boolean;
    isThinking?: boolean;
    isLight?: boolean;
    isBuddy?: boolean;
};
type RichModelSelectItemProps = RichModelData;
export declare const RichModelSelectItem: React.FC<RichModelSelectItemProps>;
export {};

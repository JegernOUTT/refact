import type { CapCost, CapsResponse } from "../services/refact";
import type { ModelCapabilities } from "../features/Providers/ProviderForm/ProviderModelsList/utils/groupModelsWithPricing";
export type EnrichedModel = {
    value: string;
    displayName: string;
    disabled: boolean;
    pricing?: CapCost;
    nCtx?: number;
    capabilities?: ModelCapabilities;
    isDefault?: boolean;
    isChat2?: boolean;
    isTaskPlannerAgent?: boolean;
    isThinking?: boolean;
    isLight?: boolean;
    isBuddy?: boolean;
    provider: string;
};
export type ModelGroup = {
    provider: string;
    displayName: string;
    models: EnrichedModel[];
};
type UsableModel = {
    value: string;
    textValue: string;
    disabled: boolean;
};
export declare function pricingForModel(pricing: Record<string, CapCost | undefined> | undefined, modelKey: string, displayName: string): CapCost | undefined;
export declare function enrichModels(usableModels: UsableModel[], caps: CapsResponse | undefined): EnrichedModel[];
export declare function groupModelsByProvider(models: EnrichedModel[]): ModelGroup[];
export declare function enrichAndGroupModels(usableModels: UsableModel[], caps: CapsResponse | undefined): ModelGroup[];
export {};

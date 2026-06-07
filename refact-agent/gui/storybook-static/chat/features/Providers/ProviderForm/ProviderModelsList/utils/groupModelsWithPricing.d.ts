import type { SimplifiedModel, ModelType } from "../../../../../services/refact";
import type { CapsResponse, CapCost } from "../../../../../services/refact";
export type UiModel = SimplifiedModel & {
    modelType: ModelType;
    pricing?: CapCost;
    pricingLabel?: string;
    nCtx?: number;
    nCtxLabel?: string;
    isDefault?: boolean;
    isChat2?: boolean;
    isTaskPlannerAgent?: boolean;
    isLight?: boolean;
    isThinking?: boolean;
    isBuddy?: boolean;
    capabilities?: ModelCapabilities;
};
export type ModelCapabilities = {
    supportsTools?: boolean;
    supportsMultimodality?: boolean;
    supportsClicks?: boolean;
    supportsAgent?: boolean;
    reasoningEffortOptions?: string[] | null;
    supportsThinkingBudget?: boolean;
    supportsAdaptiveThinkingBudget?: boolean;
};
export type ModelGroup = {
    id: string;
    title: string;
    description?: string;
    models: UiModel[];
};
/**
 * Format context window size to human-readable format
 */
export declare function formatContextWindow(nCtx: number): string;
export declare function formatPricing(cost: CapCost, compact?: boolean): string;
/**
 * Attach pricing, context window & capability flags to each simplified model.
 * Works even if caps/metadata/pricing is missing.
 */
export declare function attachPricingAndCapabilities(models: SimplifiedModel[], { caps, modelType, providerName, }: {
    caps?: CapsResponse;
    modelType: ModelType;
    providerName?: string;
}): UiModel[];
/**
 * Group models for UI. Uses default / thinking / light groups when possible.
 * Falls back to a single group if there's no useful structure.
 */
export declare function groupModelsWithPricing(models: SimplifiedModel[], options: {
    caps?: CapsResponse;
    modelType: ModelType;
    providerName?: string;
}): ModelGroup[];

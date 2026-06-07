import { type FC } from "react";
import type { ModelType } from "../../../../services/refact";
import type { UiModel } from "./utils/groupModelsWithPricing";
export type ModelCardProps = {
    model: UiModel;
    providerName: string;
    modelType: ModelType;
    isReadonlyProvider: boolean;
    currentModelNames: string[];
};
/**
 * Card component that displays model information and provides access to model settings
 */
export declare const ModelCard: FC<ModelCardProps>;

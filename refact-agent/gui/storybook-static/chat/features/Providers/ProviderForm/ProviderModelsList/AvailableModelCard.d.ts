import { type FC } from "react";
import type { AvailableModel } from "../../../../services/refact";
export type AvailableModelCardProps = {
    model: AvailableModel;
    providerName: string;
    baseProvider: string;
    isReadonlyProvider: boolean;
    onEditModel?: (model: AvailableModel) => void;
};
/**
 * Card component that displays an available model with enable/disable toggle
 */
export declare const AvailableModelCard: FC<AvailableModelCardProps>;

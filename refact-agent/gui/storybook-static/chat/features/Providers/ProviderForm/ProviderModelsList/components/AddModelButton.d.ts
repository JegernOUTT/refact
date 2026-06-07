import type { FC } from "react";
import { ModelType } from "../../../../../services/refact";
export type AddModelButtonProps = {
    modelType: ModelType;
    providerName: string;
    currentModelNames: string[];
};
export declare const AddModelButton: FC<AddModelButtonProps>;

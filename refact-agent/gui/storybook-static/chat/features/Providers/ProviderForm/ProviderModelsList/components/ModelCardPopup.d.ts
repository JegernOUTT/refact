import type { FC } from "react";
import type { Model, ModelType, SimplifiedModel } from "../../../../../services/refact";
export type ModelCardPopupProps = {
    minifiedModel?: SimplifiedModel;
    isOpen: boolean;
    isSaving: boolean;
    setIsOpen: (state: boolean) => void;
    onSave: (model: Model) => Promise<boolean>;
    onUpdate: ({ model, oldModel, }: {
        model: Model;
        oldModel: SimplifiedModel;
    }) => Promise<boolean>;
    modelName: string;
    modelType: ModelType;
    providerName: string;
    currentModelNames: string[];
    newModelCreation?: boolean;
    isRemovable?: boolean;
};
export declare const ModelCardPopup: FC<ModelCardPopupProps>;

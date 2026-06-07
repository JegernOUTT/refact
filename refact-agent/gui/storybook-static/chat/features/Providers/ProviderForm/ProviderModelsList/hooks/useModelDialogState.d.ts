import { Dispatch, SetStateAction } from 'react';
import type { Model, ModelType, SimplifiedModel } from "../../../../../services/refact";
/**
 * Custom hook for managing model dialog state with body style reset functionality
 */
export declare const useModelDialogState: ({ modelType, providerName, initialState, }: {
    modelType: ModelType;
    providerName: string;
    initialState?: boolean;
}) => {
    isOpen: boolean;
    isSavingModel: boolean;
    isRemovingModel: boolean;
    setIsRemovingModel: Dispatch<SetStateAction<boolean>>;
    setIsSavingModel: Dispatch<SetStateAction<boolean>>;
    setIsOpen: (state: boolean) => void;
    dropdownOpen: boolean;
    setDropdownOpen: Dispatch<SetStateAction<boolean>>;
    openDialogSafely: () => void;
    resetBodyStyles: () => void;
    handleSaveModel: (modelData: Model) => Promise<boolean>;
    handleRemoveModel: ({ model, operationType, isSilent, }: {
        model: SimplifiedModel;
        operationType?: "remove" | "reset";
        isSilent?: boolean;
    }) => Promise<boolean>;
    handleResetModel: (model: SimplifiedModel) => Promise<void>;
    handleUpdateModel: ({ model, oldModel, }: {
        model: Model;
        oldModel: SimplifiedModel;
    }) => Promise<boolean>;
    handleToggleModelEnabledState: (model: SimplifiedModel) => Promise<void>;
};

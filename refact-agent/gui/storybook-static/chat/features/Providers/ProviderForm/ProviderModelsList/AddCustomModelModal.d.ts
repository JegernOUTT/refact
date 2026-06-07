import { type FC } from "react";
import { type AvailableModel } from "../../../../services/refact";
export type AddCustomModelModalProps = {
    providerName: string;
    isOpen: boolean;
    onClose: () => void;
    initialModel?: AvailableModel;
    isEditingCustomModel?: boolean;
};
export declare const AddCustomModelModal: FC<AddCustomModelModalProps>;

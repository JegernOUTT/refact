import React from "react";
import type { ProviderListItem } from "../../../services/refact";
export type AddProviderInstanceModalProps = {
    isOpen: boolean;
    configuredProviders: ProviderListItem[];
    initialBaseProvider: string | null;
    onOpenChange: (open: boolean) => void;
    onCreated: (provider: ProviderListItem) => void;
};
export declare const AddProviderInstanceModal: React.FC<AddProviderInstanceModalProps>;

import React from "react";
import type { ProviderListItem } from "../../../services/refact";
export type ProviderFormProps = {
    currentProvider: ProviderListItem;
};
export type { ProviderListItem };
export declare const ProviderForm: React.FC<ProviderFormProps>;

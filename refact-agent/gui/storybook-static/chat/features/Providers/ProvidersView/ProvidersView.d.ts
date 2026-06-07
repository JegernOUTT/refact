import React from "react";
import type { ProviderListItem } from "../../../services/refact";
export type ProvidersViewProps = {
    configuredProviders: ProviderListItem[];
    backFromProviders: () => void;
};
export declare const ProvidersView: React.FC<ProvidersViewProps>;

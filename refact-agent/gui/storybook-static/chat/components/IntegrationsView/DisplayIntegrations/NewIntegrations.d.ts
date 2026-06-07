import { FC } from "react";
import { IntegrationWithIconRecord, NotConfiguredIntegrationWithIconRecord } from "../../../services/refact";
type NewIntegrationsProps = {
    availableIntegrationsToConfigure?: NotConfiguredIntegrationWithIconRecord[];
    handleIntegrationShowUp: (integration: IntegrationWithIconRecord | NotConfiguredIntegrationWithIconRecord) => void;
};
export declare const NewIntegrations: FC<NewIntegrationsProps>;
export {};

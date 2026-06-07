import { FC } from "react";
import { IntegrationWithIconRecord, NotConfiguredIntegrationWithIconRecord } from "../../../services/refact";
type GlobalIntegrationsProps = {
    globalIntegrations?: IntegrationWithIconRecord[];
    handleIntegrationShowUp: (integration: IntegrationWithIconRecord | NotConfiguredIntegrationWithIconRecord) => void;
};
export declare const GlobalIntegrations: FC<GlobalIntegrationsProps>;
export {};

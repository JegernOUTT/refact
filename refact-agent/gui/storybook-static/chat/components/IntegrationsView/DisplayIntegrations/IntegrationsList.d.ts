import { FC } from "react";
import { IntegrationWithIconRecord, NotConfiguredIntegrationWithIconRecord } from "../../../services/refact";
type IntegrationsListProps = {
    globalIntegrations?: IntegrationWithIconRecord[];
    groupedProjectIntegrations?: Record<string, IntegrationWithIconRecord[]>;
    availableIntegrationsToConfigure?: NotConfiguredIntegrationWithIconRecord[];
    handleIntegrationShowUp: (integration: IntegrationWithIconRecord | NotConfiguredIntegrationWithIconRecord) => void;
};
export declare const IntegrationsList: FC<IntegrationsListProps>;
export {};

import { FC } from "react";
import { IntegrationWithIconRecord, NotConfiguredIntegrationWithIconRecord } from "../../../services/refact";
type ProjectIntegrationsProps = {
    groupedProjectIntegrations?: Record<string, IntegrationWithIconRecord[]>;
    handleIntegrationShowUp: (integration: IntegrationWithIconRecord | NotConfiguredIntegrationWithIconRecord) => void;
};
export declare const ProjectIntegrations: FC<ProjectIntegrationsProps>;
export {};

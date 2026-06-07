import type { FC } from "react";
import { IntegrationWithIconRecord, NotConfiguredIntegrationWithIconRecord } from "../../../services/refact";
type IntegrationCardProps = {
    integration: IntegrationWithIconRecord | NotConfiguredIntegrationWithIconRecord;
    handleIntegrationShowUp: (integration: IntegrationWithIconRecord | NotConfiguredIntegrationWithIconRecord) => void;
    isNotConfigured?: boolean;
};
export declare const IntegrationCard: FC<IntegrationCardProps>;
export {};

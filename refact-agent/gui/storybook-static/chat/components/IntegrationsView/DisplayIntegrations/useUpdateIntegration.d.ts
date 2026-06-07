import { IntegrationWithIconRecord, NotConfiguredIntegrationWithIconRecord } from "../../../services/refact";
export declare const useUpdateIntegration: ({ integration, }: {
    integration: IntegrationWithIconRecord | NotConfiguredIntegrationWithIconRecord;
}) => {
    updateIntegrationAvailability: () => Promise<void>;
    integrationAvailability: Record<string, boolean>;
    isUpdatingAvailability: boolean;
};

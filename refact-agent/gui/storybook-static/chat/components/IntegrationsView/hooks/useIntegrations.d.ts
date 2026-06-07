import { FormEvent } from "react";
import { IntegrationField, IntegrationPrimitive, SchemaToolConfirmation, SmartLink, Integration, IntegrationFieldValue, IntegrationWithIconRecord, IntegrationWithIconResponse, NotConfiguredIntegrationWithIconRecord } from '../../../services/refact';
type useIntegrationsViewArgs = {
    integrationsMap?: IntegrationWithIconResponse;
    handleIfInnerIntegrationWasSet: (state: boolean) => void;
    goBack?: () => void;
};
export declare const INTEGRATIONS_WITH_TERMINAL_ICON: string[];
export declare const useIntegrations: ({ integrationsMap, handleIfInnerIntegrationWasSet, goBack, }: useIntegrationsViewArgs) => {
    currentIntegration: IntegrationWithIconRecord | null;
    currentIntegrationSchema: {
        description?: string;
        fields: Record<string, IntegrationField<NonNullable<IntegrationPrimitive>>>;
        available: Record<string, boolean>;
        confirmation: SchemaToolConfirmation;
        smartlinks?: SmartLink[];
    } | null;
    currentIntegrationValues: Record<string, IntegrationFieldValue> | null;
    currentNotConfiguredIntegration: NotConfiguredIntegrationWithIconRecord | null;
    integrationLogo: string;
    handleFormReturn: () => void;
    handleSubmit: (event: FormEvent<HTMLFormElement>) => Promise<void>;
    handleDeleteIntegration: (configurationPath: string) => Promise<void>;
    handleNotConfiguredIntegrationSubmit: (event: FormEvent<HTMLFormElement>) => void;
    handleMCPWizardSubmit: (configPath: string, integrName: string, initialInput?: {
        input: string;
        transport: string;
    }) => void;
    handleNavigateToIntegrationSetup: (integrationName: string, _integrationConfigPath: string) => void;
    handleSetCurrentIntegrationSchema: (schema: Integration["integr_schema"]) => void;
    handleSetCurrentIntegrationValues: (values: Integration["integr_values"]) => void;
    goBackAndClearError: () => void;
    handleIntegrationShowUp: (integration: IntegrationWithIconRecord | NotConfiguredIntegrationWithIconRecord) => void;
    handleUpdateFormField: (fieldKey: string, fieldValue: IntegrationFieldValue) => void;
    isDisabledIntegrationForm: boolean;
    isApplyingIntegrationForm: boolean;
    isDeletingIntegration: boolean;
    globalIntegrations: IntegrationWithIconRecord[] | undefined;
    groupedProjectIntegrations: Record<string, IntegrationWithIconRecord[]> | undefined;
    availableIntegrationsToConfigure: NotConfiguredIntegrationWithIconRecord[] | undefined;
    formValues: Record<string, IntegrationFieldValue> | null;
};
export {};

import { FC } from "react";
import { NotConfiguredIntegrationWithIconRecord } from "../../../services/refact";
type MCPSetupWizardProps = {
    integration: NotConfiguredIntegrationWithIconRecord;
    onSubmit: (configPath: string, integrName: string, initialInput?: {
        input: string;
        transport: string;
    }) => void;
};
export declare const MCPSetupWizard: FC<MCPSetupWizardProps>;
export {};

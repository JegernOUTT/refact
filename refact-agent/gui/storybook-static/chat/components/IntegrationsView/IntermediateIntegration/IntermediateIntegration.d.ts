import { type FormEvent, type FC } from "react";
import { NotConfiguredIntegrationWithIconRecord } from "../../../services/refact";
type IntegrationCmdlineProps = {
    integration: NotConfiguredIntegrationWithIconRecord;
    handleSubmit: (event: FormEvent<HTMLFormElement>) => void;
    handleMCPWizardSubmit?: (configPath: string, integrName: string) => void;
};
export declare const IntermediateIntegration: FC<IntegrationCmdlineProps>;
export {};

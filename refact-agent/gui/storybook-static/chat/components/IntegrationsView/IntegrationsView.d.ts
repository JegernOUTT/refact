import { FC } from "react";
import { LeftRightPadding } from "../../features/Integrations/Integrations";
import { IntegrationWithIconResponse } from "../../services/refact";
type IntegrationViewProps = {
    integrationsMap?: IntegrationWithIconResponse;
    leftRightPadding: LeftRightPadding;
    isLoading: boolean;
    goBack?: () => void;
    handleIfInnerIntegrationWasSet: (state: boolean) => void;
};
export declare const IntegrationsView: FC<IntegrationViewProps>;
export {};

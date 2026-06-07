import type { FC } from "react";
import { LeftRightPadding } from "../../../features/Integrations/Integrations.tsx";
type IntegrationsHeaderProps = {
    handleFormReturn: () => void;
    integrationName: string;
    leftRightPadding: LeftRightPadding;
    icon: string;
    instantBackReturn?: boolean;
    handleInstantReturn?: () => void;
};
export declare const IntegrationsHeader: FC<IntegrationsHeaderProps>;
export {};

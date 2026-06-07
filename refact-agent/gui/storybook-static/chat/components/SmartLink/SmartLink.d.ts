import type { FC } from "react";
import type { SmartLink as SmartLinkType } from "../../services/refact";
export declare const SmartLink: FC<{
    smartlink: SmartLinkType;
    integrationName: string;
    integrationPath: string;
    integrationProject: string;
    isSmall?: boolean;
    shouldBeDisabled?: boolean;
}>;

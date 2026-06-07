import React, { MouseEventHandler } from "react";
export type OnOffSwitchProps = {
    isEnabled: boolean;
    isUnavailable?: boolean;
    isUpdating?: boolean;
    handleClick: MouseEventHandler<HTMLDivElement>;
};
export declare const OnOffSwitch: React.FC<OnOffSwitchProps>;

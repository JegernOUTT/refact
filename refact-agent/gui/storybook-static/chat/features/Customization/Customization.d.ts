import React from "react";
import { ConfigItem, ConfigKind } from "../../services/refact/customization";
import type { Config } from "../Config/configSlice";
export type CustomizationProps = {
    backFromCustomization: () => void;
    host: Config["host"];
    tabbed: Config["tabbed"];
    initialKind?: ConfigKind;
    initialConfigId?: string;
    draftId?: string;
};
export declare const ConfigEditor: React.FC<{
    kind: ConfigKind;
    configId: string;
    configItem: ConfigItem;
    onSaved: () => void;
    draftId?: string;
}>;
export declare const Customization: React.FC<CustomizationProps>;

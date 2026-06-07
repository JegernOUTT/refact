import { FC } from "react";
import { Integration, SmartLink as TSmartLink } from "../../../services/refact";
type FormSmartlinksProps = {
    integration: Integration;
    smartlinks: TSmartLink[] | undefined;
};
export declare const FormSmartlinks: FC<FormSmartlinksProps>;
export {};

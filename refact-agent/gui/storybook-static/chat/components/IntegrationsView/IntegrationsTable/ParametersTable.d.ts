import { FC } from "react";
import { ToolParameterEntity } from "../../../services/refact";
type ParametersTableProps = {
    initialData: ToolParameterEntity[];
    onToolParameters: (data: ToolParameterEntity[]) => void;
};
export declare const ParametersTable: FC<ParametersTableProps>;
export {};

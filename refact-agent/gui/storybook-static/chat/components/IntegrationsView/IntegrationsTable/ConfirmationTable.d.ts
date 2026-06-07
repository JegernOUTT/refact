import { FC } from "react";
type ConfirmationTableProps = {
    tableName: string;
    initialData: string[];
    onToolConfirmation: (key: string, data: string[]) => void;
};
export declare const ConfirmationTable: FC<ConfirmationTableProps>;
export {};

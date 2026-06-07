import { FC } from "react";
type KeyValueTableProps = {
    initialData: Record<string, string>;
    onChange: (data: Record<string, string>) => void;
    columnNames?: string[];
    emptyMessage?: string;
};
export declare const KeyValueTable: FC<KeyValueTableProps>;
export {};

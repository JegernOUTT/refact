import React from "react";
export type SchemaFieldDef = {
    key: string;
    f_type: string;
    f_desc?: string;
    f_label?: string;
    f_placeholder?: string;
    f_default?: string;
    f_extra?: boolean;
    f_secret?: boolean;
    smartlinks?: {
        sl_label: string;
        sl_goto: string;
    }[];
};
export type SchemaFieldProps = {
    field: SchemaFieldDef;
    value: unknown;
    disabled?: boolean;
    onSave: (key: string, value: unknown) => Promise<void>;
};
export declare const SchemaField: React.FC<SchemaFieldProps>;

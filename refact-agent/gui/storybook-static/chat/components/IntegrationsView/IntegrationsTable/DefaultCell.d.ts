import { JSX } from 'react/jsx-runtime';
import type { KeyboardEvent } from "react";
type DefaultCellProps = {
    initialValue: string;
    onChange: (value: string) => void;
    onKeyPress: (e: KeyboardEvent<HTMLInputElement>) => void;
    "data-row-index"?: number;
    "data-field"?: string;
    "data-next-row"?: string;
};
export declare const DefaultCell: ({ initialValue, onChange, onKeyPress, "data-row-index": dataRowIndex, "data-field": dataField, "data-next-row": dataNextRow, }: DefaultCellProps) => JSX.Element;
export {};

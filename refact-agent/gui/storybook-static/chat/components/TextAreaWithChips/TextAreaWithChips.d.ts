import React from "react";
import { type TextAreaProps } from "../TextArea/TextArea";
type TextAreaWithChipsProps = TextAreaProps & {
    host: string;
    onOpenFile?: (file: {
        file_path: string;
        line?: number;
    }) => Promise<void>;
};
export declare const TextAreaWithChips: React.ForwardRefExoticComponent<Omit<TextAreaWithChipsProps, "ref"> & React.RefAttributes<HTMLTextAreaElement>>;
export {};

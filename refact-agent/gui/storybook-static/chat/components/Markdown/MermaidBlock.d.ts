import React from "react";
export type MermaidBlockProps = {
    code: string;
    onCopyClick?: (str: string) => void;
};
export declare const MermaidBlock: React.NamedExoticComponent<MermaidBlockProps>;

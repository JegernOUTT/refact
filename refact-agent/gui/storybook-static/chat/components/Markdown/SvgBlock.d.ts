import React from "react";
export type SvgBlockProps = {
    code: string;
    onCopyClick?: (str: string) => void;
};
export declare const SvgBlock: React.NamedExoticComponent<SvgBlockProps>;

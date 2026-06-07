import React, { CSSProperties } from "react";
import { CodeProps } from "@radix-ui/themes";
import type { Element } from "hast";
export type MarkdownControls = {
    onCopyClick: (str: string) => void;
};
export type ShikiCodeBlockProps = React.JSX.IntrinsicElements["code"] & {
    node?: Element | undefined;
    style?: CSSProperties;
    wrap?: boolean;
    preOptions?: {
        noMargin?: boolean;
        widthMaxContent?: boolean;
    };
    color?: CodeProps["color"];
    showLineNumbers?: boolean;
    isStreaming?: boolean;
} & Partial<MarkdownControls>;
export declare const ShikiCodeBlock: React.NamedExoticComponent<ShikiCodeBlockProps>;

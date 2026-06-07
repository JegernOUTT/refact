import React from "react";
export interface ArtifactBlockProps {
    code: string;
    isStreaming?: boolean;
    onCopyClick?: (str: string) => void;
}
export declare const ArtifactBlock: React.NamedExoticComponent<ArtifactBlockProps>;

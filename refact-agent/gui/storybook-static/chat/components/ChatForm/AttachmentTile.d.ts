import React from "react";
export type AttachmentTileProps = {
    kind: "image";
    id: string;
    name: string;
    src: string;
    onRemove?: () => void;
} | {
    kind: "file";
    id: string;
    name: string;
    copyText: string;
    subtitle?: string;
    onRemove?: () => void;
    onOpen?: () => void | Promise<void>;
} | {
    kind: "plain-text";
    id: string;
    label: string;
    preview: string;
    copyText: string;
};
export declare const AttachmentTile: React.FC<AttachmentTileProps>;

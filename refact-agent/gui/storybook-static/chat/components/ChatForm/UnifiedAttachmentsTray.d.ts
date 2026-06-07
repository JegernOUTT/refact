import React from "react";
import { useAttachedFiles } from "./useCheckBoxes";
import { ChatContextFile } from "../../services/refact";
import type { ManualPreviewItem } from "../../features/Chat/Thread/types";
type UnifiedAttachmentsTrayProps = {
    attachedFiles: ReturnType<typeof useAttachedFiles>;
    previewFiles?: (ChatContextFile | string)[];
    manualPreviewItems?: ManualPreviewItem[];
    onRemoveManualPreviewItem?: (index: number) => void;
    onOpenFile?: (file: {
        file_path: string;
        line?: number;
    }) => void | Promise<void>;
};
export declare const UnifiedAttachmentsTray: React.FC<UnifiedAttachmentsTrayProps>;
export {};

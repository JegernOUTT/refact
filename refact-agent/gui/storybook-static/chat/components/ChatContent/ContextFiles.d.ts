import React from "react";
import { ChatContextFile } from "../../services/refact";
export declare const Markdown: React.FC<{
    children: string;
}>;
export declare const ContextFiles: React.NamedExoticComponent<{
    files: ChatContextFile[];
    toolCallId?: string;
    open?: boolean;
    onOpenChange?: (open: boolean) => void;
}>;

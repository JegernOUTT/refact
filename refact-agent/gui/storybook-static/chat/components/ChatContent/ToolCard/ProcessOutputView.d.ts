import React from "react";
import type { ExecTranscriptMetadata } from "../../../services/refact/types";
type ProcessOutputViewProps = {
    content: string | null;
    transcript?: ExecTranscriptMetadata;
};
export declare const ProcessOutputView: React.FC<ProcessOutputViewProps>;
export default ProcessOutputView;

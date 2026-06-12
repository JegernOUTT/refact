import { BarChart3, Archive } from "lucide-react";
import React from "react";
import { ToolCall } from "../../../services/refact/types";
import { ReportToolCard } from "./ReportToolCard";
import {
  extractApplyReport,
  extractProbeReport,
} from "./compressReportParsers";

interface CompressReportToolProps {
  toolCall: ToolCall;
  toolType: "ctx_probe" | "ctx_apply";
}

export const CompressReportTool: React.FC<CompressReportToolProps> = ({
  toolCall,
  toolType,
}) => {
  const isProbe = toolType === "ctx_probe";

  return (
    <ReportToolCard
      toolCall={toolCall}
      icon={isProbe ? <BarChart3 /> : <Archive />}
      defaultSummary={isProbe ? "Analyze chat" : "Compress chat"}
      extractReport={isProbe ? extractProbeReport : extractApplyReport}
      unboundedContent
    />
  );
};

export default CompressReportTool;

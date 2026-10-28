import { Archive, BarChart3 } from "lucide-react";
import React, { useMemo } from "react";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { selectToolResultByThreadAndId } from "../../../features/Chat/Thread/selectors";
import { useThreadId } from "../../../features/Chat/Thread";
import type { ToolCall } from "../../../services/refact/types";
import { Markdown } from "../../Markdown";
import {
  TranscriptClamp,
  TranscriptItem,
  type TranscriptStatus,
} from "../TranscriptItem";
import { ToolCallTooltip } from "./ToolCallTooltip";
import {
  extractApplyReport,
  extractProbeReport,
} from "./compressReportParsers";
import styles from "./GenericTool.module.css";

interface CompressReportToolProps {
  toolCall: ToolCall;
  toolType: "ctx_probe" | "ctx_apply";
}

export const CompressReportTool: React.FC<CompressReportToolProps> = ({
  toolCall,
  toolType,
}) => {
  const threadId = useThreadId();
  const maybeResult = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );
  const isProbe = toolType === "ctx_probe";
  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;
  const failed = Boolean(maybeResult?.tool_failed);
  const report = useMemo(() => {
    if (!content) return null;
    return isProbe ? extractProbeReport(content) : extractApplyReport(content);
  }, [content, isProbe]);

  const status: TranscriptStatus = !maybeResult
    ? "running"
    : failed
      ? "failed"
      : "done";
  const markdown = report?.markdown ?? (failed ? null : content);

  return (
    <ToolCallTooltip toolCall={toolCall}>
      <TranscriptItem
        className="rf-enter"
        data-testid="compress-report-tool"
        error={failed ? content : null}
        icon={isProbe ? <BarChart3 /> : <Archive />}
        meta={report?.meta}
        openKey={toolCall.id ? `tc:${toolCall.id}` : undefined}
        output={
          markdown ? (
            <TranscriptClamp>
              <div className={styles.markdownContent}>
                <Markdown>{markdown}</Markdown>
              </div>
            </TranscriptClamp>
          ) : null
        }
        raw={[{ label: "Arguments", value: toolCall.function.arguments }]}
        status={status}
        target={report?.summary ?? (isProbe ? "Analyze chat" : "Compress chat")}
        verb="Compress"
      />
    </ToolCallTooltip>
  );
};

export default CompressReportTool;

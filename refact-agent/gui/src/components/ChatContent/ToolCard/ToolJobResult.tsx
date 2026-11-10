import { useState } from "react";
import { CheckCheck } from "lucide-react";
import type { NativeToolJobResult } from "../ChatContentDisplayItems";
import type { ToolResult } from "../../../services/refact/types";
import { ContextFiles } from "../ContextFiles";
import { DialogImage } from "../../DialogImage";
import { buildImageDataUrl } from "../../../utils/imageDataUrl";
import { ResultView } from "./GenericTool";
import { ToolCard } from "./ToolCard";
import { AppTool } from "./AppTool";
import { isAppToolName } from "./appToolModel";
import { normalizeToolName } from "../../../utils/toolNameAliases";
import { useThreadId } from "../../../features/Chat/Thread/ChatThreadContext";
import { PrivacyToolResult } from "../../../features/Privacy/PrivacyToolResult";
import {
  extractPrivacyShellMetadata,
  isPrivacyRefusalContent,
} from "../../../services/refact/privacy";
import styles from "./ToolJobResult.module.css";

// Render the delivered native body directly: never create another tool call or
// consult the original call's receipt/polling state for this completion actor.
export function ToolJobResult({
  result,
  embedded = false,
}: {
  result: NativeToolJobResult;
  embedded?: boolean;
}) {
  const threadId = useThreadId();
  const [open, setOpen] = useState(false);
  const { message, toolName, toolCall } = result;
  const toolResult: ToolResult | undefined =
    message.role !== "tool"
      ? undefined
      : typeof message.content === "string"
        ? { ...message, content: message.content }
        : { ...message, content: message.content };
  if (
    toolResult &&
    (extractPrivacyShellMetadata(toolResult.extra) !== null ||
      isPrivacyRefusalContent(toolResult.content))
  ) {
    return (
      <PrivacyToolResult
        threadId={threadId}
        toolCallId={toolResult.tool_call_id}
        result={toolResult}
      />
    );
  }
  if (toolResult && toolCall && isAppToolName(normalizeToolName(toolName))) {
    return (
      <AppTool toolCall={toolCall} result={toolResult} embedded={embedded} />
    );
  }
  const body = (
    <div className={styles.body} data-testid="tool-job-result">
      {message.role === "context_file" ? (
        <ContextFiles
          files={message.content}
          toolCallId={message.tool_call_id}
          open
        />
      ) : typeof message.content === "string" ? (
        <ResultView content={message.content} />
      ) : (
        message.content.map((part, index) => {
          if (part.m_type === "text") {
            return <ResultView key={index} content={part.m_content} />;
          }
          const src = buildImageDataUrl(part.m_type, part.m_content);
          return src ? (
            <DialogImage key={index} src={src} fallback="" size="8" />
          ) : null;
        })
      )}
    </div>
  );
  if (embedded) return body;
  return (
    <ToolCard
      icon={<CheckCheck size={14} />}
      summary={toolName}
      verb="Tool result"
      density="compact"
      status={message.role === "tool" && message.tool_failed ? "error" : "idle"}
      isOpen={open}
      onToggle={() => setOpen((previous) => !previous)}
    >
      {body}
    </ToolCard>
  );
}

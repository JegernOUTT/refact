import { CheckCheck } from "lucide-react";
import type { NativeToolJobResult } from "../ChatContentDisplayItems";
import { ContextFiles } from "../ContextFiles";
import { DialogImage } from "../../DialogImage";
import { buildImageDataUrl } from "../../../utils/imageDataUrl";
import { ResultView } from "./GenericTool";
import { ToolCard } from "./ToolCard";
import styles from "./ToolJobResult.module.css";

// Render the delivered native body directly: never create another tool call or
// consult the original call's receipt/polling state for this completion actor.
export function ToolJobResult({ result }: { result: NativeToolJobResult }) {
  const { message, toolName } = result;
  return (
    <ToolCard
      icon={<CheckCheck size={14} />}
      summary={toolName}
      verb="Tool result"
      density="compact"
      status={message.role === "tool" && message.tool_failed ? "error" : "idle"}
      isOpen
      onToggle={() => undefined}
      collapsible={false}
    >
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
    </ToolCard>
  );
}

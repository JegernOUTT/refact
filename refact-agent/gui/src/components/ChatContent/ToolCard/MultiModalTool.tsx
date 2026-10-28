import { Image as ImageIcon } from "lucide-react";
import React, { useMemo } from "react";
import type {
  MultiModalToolResult,
  ToolCall,
} from "../../../services/refact/types";
import { DialogImage } from "../../DialogImage";
import { buildImageDataUrl } from "../../../utils/imageDataUrl";
import { TranscriptItem, type TranscriptRawBlock } from "../TranscriptItem";
import { ToolCallTooltip } from "./ToolCallTooltip";
import { ArgumentsView, ResultView } from "./GenericTool";
import { describeToolCall, parseToolArgs } from "./genericToolTitle";
import styles from "./MultiModalTool.module.css";

interface MultiModalToolProps {
  toolCall: ToolCall;
  result: MultiModalToolResult;
}

function imageCountLabel(count: number): string | undefined {
  if (count === 0) return undefined;
  return count === 1 ? "1 image" : `${count} images`;
}

export const MultiModalTool: React.FC<MultiModalToolProps> = ({
  toolCall,
  result,
}) => {
  const rawArguments = toolCall.function.arguments;
  const args = useMemo(() => parseToolArgs(rawArguments), [rawArguments]);
  const title = useMemo(
    () => describeToolCall(toolCall.function.name, rawArguments),
    [toolCall.function.name, rawArguments],
  );
  const images = useMemo(
    () =>
      result.content.flatMap((part) => {
        const url = buildImageDataUrl(part.m_type, part.m_content);
        return url ? [url] : [];
      }),
    [result.content],
  );
  const text = useMemo(
    () =>
      result.content
        .filter((part) => part.m_type === "text")
        .map((part) => part.m_content)
        .join("\n\n")
        .trim(),
    [result.content],
  );
  const failed = result.tool_failed === true;
  const raw: TranscriptRawBlock[] = [
    { label: "Arguments", value: rawArguments },
    { label: "Response", value: result.content },
  ];

  return (
    <ToolCallTooltip toolCall={toolCall}>
      <TranscriptItem
        className="rf-enter"
        data-testid="multimodal-tool"
        artifacts={
          images.length > 0 ? (
            <div className={styles.images} data-testid="multimodal-images">
              {images.map((src, index) => (
                <DialogImage
                  fallback=""
                  key={`${toolCall.id ?? "call"}-${index}`}
                  size="8"
                  src={src}
                />
              ))}
            </div>
          ) : null
        }
        defaultOpen={images.length > 0}
        icon={<ImageIcon />}
        input={
          Object.keys(args).length > 0 ? <ArgumentsView args={args} /> : null
        }
        meta={imageCountLabel(images.length)}
        openKey={toolCall.id ? `tc:${toolCall.id}` : undefined}
        output={text ? <ResultView content={text} /> : null}
        raw={raw}
        status={failed ? "failed" : "done"}
        target={title.target ?? title.meta}
        verb={title.verb}
      />
    </ToolCallTooltip>
  );
};

export default MultiModalTool;

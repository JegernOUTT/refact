import React from "react";
import type { ToolCall } from "../../../services/refact/types";
import { DialogImage } from "../../DialogImage";
import { Markdown, ShikiCodeBlock } from "../../Markdown";
import {
  TranscriptClamp,
  TranscriptItem,
  type TranscriptFact,
} from "../TranscriptItem";
import { ToolCallTooltip } from "./ToolCallTooltip";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";
import styles from "./OpenAIResponsesTool.module.css";

export interface OpenAIServerToolProps {
  toolCall: ToolCall;
  icon: React.ReactNode;
  verb: string;
  target?: string;
  meta?: string;
  summary?: readonly TranscriptFact[];
  output?: React.ReactNode;
  images?: readonly string[];
}

export function ServerToolText({ text }: { text: string }) {
  return (
    <TranscriptClamp lineCount={text.split("\n").length}>
      <ShikiCodeBlock showLineNumbers={false}>{text}</ShikiCodeBlock>
    </TranscriptClamp>
  );
}

export function OpenAIServerTool({
  toolCall,
  icon,
  verb,
  target,
  meta,
  summary,
  output,
  images = [],
}: OpenAIServerToolProps) {
  const state = useOpenAiResponsesToolCardState(toolCall);
  const content = state.contentText ? (
    <TranscriptClamp>
      <div className={styles.content}>
        <Markdown>{state.contentText}</Markdown>
      </div>
    </TranscriptClamp>
  ) : null;
  const body =
    output ?? content ? (
      <div className={styles.content}>
        {content}
        {output}
      </div>
    ) : null;
  return (
    <ToolCallTooltip toolCall={toolCall}>
      <TranscriptItem
        artifacts={
          images.length > 0 ? (
            <div className={styles.imageGrid}>
              {images.map((url, index) => (
                <DialogImage key={index} src={url} fallback="" size="8" />
              ))}
            </div>
          ) : null
        }
        className="rf-enter"
        data-status={state.status}
        data-testid="openai-server-tool"
        icon={icon}
        meta={meta}
        openKey={toolCall.id ? `tc:${toolCall.id}` : undefined}
        output={body}
        raw={[
          { label: "Item JSON", value: state.rawJson },
          ...(state.contentText
            ? [{ label: "Response", value: state.contentText }]
            : []),
        ]}
        status={state.transcriptStatus}
        statusLabel={state.statusLabel}
        summary={summary}
        target={target}
        verb={verb}
      />
    </ToolCallTooltip>
  );
}

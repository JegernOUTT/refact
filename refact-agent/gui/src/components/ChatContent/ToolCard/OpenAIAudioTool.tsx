import React, { useMemo } from "react";
import { SpeakerLoudIcon } from "@radix-ui/react-icons";
import { Box, Text } from "@radix-ui/themes";

import { ToolCard } from "./ToolCard";
import type { ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import styles from "./OpenAIResponsesTool.module.css";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";

type Props = {
  toolCall: ToolCall;
};

export const OpenAIAudioTool: React.FC<Props> = ({ toolCall }) => {
  const state = useOpenAiResponsesToolCardState(toolCall);
  const args = state.parsedArgs as Record<string, unknown> | null;

  const transcript = useMemo(() => {
    if (!args) return null;
    if (typeof args.transcript === "string") return args.transcript;
    if (typeof args.text === "string") return args.text;
    return null;
  }, [args]);

  const summary = transcript ? (
    <>
      Audio:{" "}
      <span className={styles.inlineCode}>
        {transcript.slice(0, 40)}
        {transcript.length > 40 ? "…" : ""}
      </span>
    </>
  ) : (
    "Audio"
  );

  return (
    <ToolCard
      icon={<SpeakerLoudIcon />}
      summary={summary}
      status={state.status}
      isOpen={state.isOpen}
      onToggle={state.toggleOpen}
      toolCall={toolCall}
    >
      {transcript && (
        <Box className={styles.codeBox}>
          <ShikiCodeBlock showLineNumbers={false}>{transcript}</ShikiCodeBlock>
        </Box>
      )}

      <Text size="1" color="gray">
        Raw JSON
      </Text>
      <ShikiCodeBlock showLineNumbers={false}>{state.rawJson}</ShikiCodeBlock>
    </ToolCard>
  );
};

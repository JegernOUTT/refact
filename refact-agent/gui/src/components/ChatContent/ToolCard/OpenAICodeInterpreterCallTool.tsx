import React, { useMemo } from "react";
import { LapTimerIcon } from "@radix-ui/react-icons";
import { Box, Flex, Text } from "@radix-ui/themes";

import { ToolCard } from "./ToolCard";
import type { ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import { DialogImage } from "../../DialogImage";
import styles from "./OpenAIResponsesTool.module.css";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";

type Props = {
  toolCall: ToolCall;
};

type CodeInterpreterOutput = {
  type?: string;
  text?: string;
  image_url?: string;
  image?: { url?: string };
};

export const OpenAICodeInterpreterCallTool: React.FC<Props> = ({
  toolCall,
}) => {
  const state = useOpenAiResponsesToolCardState(toolCall);
  const args = state.parsedArgs as Record<string, unknown> | null;

  const outputs = useMemo(() => {
    if (!args) return [] as CodeInterpreterOutput[];
    if (!Array.isArray(args.outputs)) return [] as CodeInterpreterOutput[];
    return (args.outputs as unknown[])
      .map((o) =>
        typeof o === "object" && o ? (o as CodeInterpreterOutput) : {},
      )
      .slice(0, 200);
  }, [args]);

  const textOutputs = outputs
    .map((o) => (typeof o.text === "string" ? o.text : null))
    .filter((t): t is string => !!t);

  const imageUrls: string[] = outputs
    .map((o) => {
      if (typeof o.image_url === "string") return o.image_url;
      if (o.image && typeof o.image.url === "string") return o.image.url;
      return null;
    })
    .filter((u): u is string => !!u);

  const summary = (
    <>
      Code Interpreter{" "}
      <span className={styles.inlineCode}>{outputs.length} outputs</span>
    </>
  );

  return (
    <ToolCard
      icon={<LapTimerIcon />}
      summary={summary}
      status={state.status}
      isOpen={state.isOpen}
      onToggle={state.toggleOpen}
      toolCall={toolCall}
    >
      {textOutputs.length > 0 && (
        <Box className={styles.codeBox}>
          <ShikiCodeBlock showLineNumbers={false}>
            {textOutputs.join("\n\n")}
          </ShikiCodeBlock>
        </Box>
      )}

      {imageUrls.length > 0 && (
        <Flex py="2" gap="2" wrap="wrap">
          {imageUrls.map((url, idx) => (
            <DialogImage key={idx} src={url} fallback="" size="8" />
          ))}
        </Flex>
      )}

      <Text size="1" color="gray">
        Raw JSON
      </Text>
      <ShikiCodeBlock showLineNumbers={false}>{state.rawJson}</ShikiCodeBlock>
    </ToolCard>
  );
};

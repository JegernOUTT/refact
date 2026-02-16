import React, { useMemo } from "react";
import { DesktopIcon, ImageIcon } from "@radix-ui/react-icons";
import { Flex, Text } from "@radix-ui/themes";

import { ToolCard } from "./ToolCard";
import type { ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import { DialogImage } from "../../DialogImage";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";

type Props = {
  toolCall: ToolCall;
};

export const OpenAIComputerCallOutputTool: React.FC<Props> = ({ toolCall }) => {
  const state = useOpenAiResponsesToolCardState(toolCall);
  const args = state.parsedArgs as Record<string, unknown> | null;

  const imageUrls = useMemo(() => {
    if (!args) return [] as string[];

    // Typical shape: { output: { image_url: "..." } }
    const output =
      typeof args.output === "object" && args.output !== null
        ? (args.output as Record<string, unknown>)
        : null;
    const url =
      output && typeof output.image_url === "string" ? output.image_url : null;

    if (url) return [url];
    return [];
  }, [args]);

  return (
    <ToolCard
      icon={imageUrls.length > 0 ? <ImageIcon /> : <DesktopIcon />}
      summary={"Computer Output"}
      status={state.status}
      isOpen={state.isOpen}
      onToggle={state.toggleOpen}
      toolCall={toolCall}
    >
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

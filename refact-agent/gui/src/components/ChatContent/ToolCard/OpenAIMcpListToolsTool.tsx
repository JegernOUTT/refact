import { Box } from "lucide-react";
import React from "react";
import { Box as RadixBox, Text } from "@radix-ui/themes";

import { ToolCard } from "./ToolCard";
import type { ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import styles from "./OpenAIResponsesTool.module.css";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";

type Props = {
  toolCall: ToolCall;
};

export const OpenAIMcpListToolsTool: React.FC<Props> = ({ toolCall }) => {
  const state = useOpenAiResponsesToolCardState(toolCall);

  return (
    <ToolCard
      icon={<Box />}
      summary={"MCP List Tools"}
      status={state.status}
      isOpen={state.isOpen}
      onToggle={state.toggleOpen}
      toolCall={toolCall}
    >
      <Text size="1" color="gray">
        Raw JSON
      </Text>
      <RadixBox className={styles.rawJson}>
        <ShikiCodeBlock showLineNumbers={false}>{state.rawJson}</ShikiCodeBlock>
      </RadixBox>
    </ToolCard>
  );
};

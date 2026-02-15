import React, { useMemo } from "react";
import { CubeIcon } from "@radix-ui/react-icons";
import { Text } from "@radix-ui/themes";

import { ToolCard } from "./ToolCard";
import type { ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import styles from "./OpenAIResponsesTool.module.css";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";

type Props = {
  toolCall: ToolCall;
};

export const OpenAIMcpCallTool: React.FC<Props> = ({ toolCall }) => {
  const state = useOpenAiResponsesToolCardState(toolCall);
  const args = state.parsedArgs as Record<string, unknown> | null;

  const summary = useMemo(() => {
    if (!args) return "MCP Call";
    const server = typeof args.server === "string" ? args.server : undefined;
    const tool = typeof args.tool === "string" ? args.tool : undefined;
    const label = [server, tool].filter(Boolean).join(" ");
    return label ? (
      <>
        MCP Call: <span className={styles.inlineCode}>{label}</span>
      </>
    ) : (
      "MCP Call"
    );
  }, [args]);

  return (
    <ToolCard
      icon={<CubeIcon />}
      summary={summary}
      status={state.status}
      isOpen={state.isOpen}
      onToggle={state.toggleOpen}
      toolCall={toolCall}
    >
      <Text size="1" color="gray">
        Raw JSON
      </Text>
      <ShikiCodeBlock showLineNumbers={false}>{state.rawJson}</ShikiCodeBlock>
    </ToolCard>
  );
};

